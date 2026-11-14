//! Lengths and angles, one type per unit and per pixel space.
//!
//! A layout measures a page in points and millimetres, lays it out in
//! document pixels, and draws it in pixels of the zoomed view. A bare float
//! lets a value of one reach a place that expects another. Each such slip
//! draws at the wrong size, and the compiler sees none of them. One type per
//! unit keeps them apart:
//!
//! | Type | Unit | Example |
//! | :--- | :--- | :--- |
//! | [`Px`] | a pixel of the space its parameter names (`crate::space`) | a column width, a pointer, a board padding |
//! | [`PrintLength`] | a point or a millimetre, as its parameter names | an indent, a page size |
//! | [`Radians`] | an angle | a rotated element |
//! | [`ViewSpeed`] | view pixels per millisecond | a flick, a glide |
//!
//! A board world unit is a pixel space, so a board length is `Px<Board>`,
//! and the `f32` an element stores is `Px<StoredBoard>`.
//!
//! No type converts into another through `From`. Each conversion is a named
//! method, or a [`Scale`] between two pixel spaces. [`Px::zoomed`] takes the
//! zoom, and [`PrintLength::to_doc_px`] applies the fixed ratio of the two
//! units. The density is the drawing surface's concern, so `ambre-canvas`
//! reads and keeps it (`CANVAS-031`).

use std::marker::PhantomData;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::font::PX_PER_PT;
use crate::scale::Scale;
use crate::space::{
    Board, CanvasCss, Css, Doc, Float, Rescale, Scalar, ScalesTo, Space, StoredBoard, Surface, View,
};
use crate::span::{FracMs, TimeSpan};

/// CSS pixels per millimetre: a CSS pixel is 1/96 inch, an inch is 25.4 mm.
pub const PX_PER_MM: f32 = 96.0 / 25.4;

/// A length, or a coordinate, in the pixel space `S` names.
///
/// The space fixes the number the length holds (`Space::Repr`), so a space
/// never holds two kinds of number. Two lengths of one space add, subtract,
/// and, for a float space, divide into a bare ratio. A float length scales
/// by a bare number of its own float. A product of two lengths does not
/// compile. A length of one space meets a length of another only through a
/// [`Scale`] or a named method.
///
/// `new` wraps the number, and `get` unwraps it for a foreign API. No rule
/// bounds a length, so the type has no fallible constructor. The serde form
/// is the bare number, as `#[serde(transparent)]` writes it.
pub struct Px<S: Space>(S::Repr, PhantomData<fn() -> S>);

impl<S: Space> Px<S> {
    /// The length of zero.
    pub const ZERO: Self = Self(<S::Repr as Scalar>::ZERO, PhantomData);

    /// The length of `value` pixels.
    #[must_use]
    pub const fn new(value: S::Repr) -> Self {
        Self(value, PhantomData)
    }

    /// The number of pixels, for a foreign API that takes the number.
    #[must_use]
    pub const fn get(self) -> S::Repr {
        self.0
    }

    /// The smaller of the two lengths.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        Self::new(self.0.lesser(other.0))
    }

    /// The larger of the two lengths.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self::new(self.0.greater(other.0))
    }

    /// The length clamped into `lo ..= hi`, never a panic: a NaN reads as
    /// `lo`, and `hi` wins when the bounds cross.
    #[must_use]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        Self::new(self.0.greater(lo.0).lesser(hi.0))
    }
}

impl<S: Space> Px<S>
where
    S::Repr: Float,
{
    /// The length without its sign.
    #[must_use]
    pub fn abs(self) -> Self {
        Self::new(self.0.magnitude())
    }

    /// The length rounded to a whole number of pixels.
    #[must_use]
    pub fn round(self) -> Self {
        Self::new(self.0.nearest())
    }

    /// The length rounded down to a whole number of pixels.
    #[must_use]
    pub fn floor(self) -> Self {
        Self::new(self.0.rounded_down())
    }

    /// Whether the length is a finite number.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.0.finite()
    }
}

impl<S: Space<Repr = i32>> Px<S> {
    /// The length without its sign, in the unsigned integer a texture
    /// coordinate takes.
    #[must_use]
    pub const fn unsigned_abs(self) -> u32 {
        self.0.unsigned_abs()
    }
}

// Every impl below is written by hand. A derive would bound the marker by
// a trait the marker has no use for.
impl<S: Space> Clone for Px<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: Space> Copy for Px<S> {}

impl<S: Space> Default for Px<S> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<S: Space> PartialEq for Px<S> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<S: Space> Eq for Px<S> where S::Repr: Eq {}

impl<S: Space> PartialOrd for Px<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl<S: Space> Ord for Px<S>
where
    S::Repr: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<S: Space> std::hash::Hash for Px<S>
where
    S::Repr: std::hash::Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// The space and the number, as `Px<Doc>(12.0)`.
impl<S: Space> std::fmt::Debug for Px<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Px<{}>({:?})", S::NAME.as_str(), self.0)
    }
}

/// The bare number, as an error message prints a dimension.
impl<S: Space> std::fmt::Display for Px<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The bare number, as `#[serde(transparent)]` writes it.
impl<S: Space> Serialize for Px<S> {
    fn serialize<W: Serializer>(&self, serializer: W) -> Result<W::Ok, W::Error> {
        self.0.serialize(serializer)
    }
}

/// The bare number, as `#[serde(transparent)]` reads it.
impl<'de, S: Space> Deserialize<'de> for Px<S> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        S::Repr::deserialize(deserializer).map(Self::new)
    }
}

impl<S: Space> Add for Px<S> {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.0 + other.0)
    }
}

impl<S: Space> Sub for Px<S> {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.0 - other.0)
    }
}

impl<S: Space> AddAssign for Px<S> {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl<S: Space> SubAssign for Px<S> {
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl<S: Space> Neg for Px<S>
where
    S::Repr: Neg<Output = S::Repr>,
{
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.0)
    }
}

// rustc refuses a `Mul<S::Repr>` beside `Mul<Scale<..>>` (E0119), so the
// bare factor is written once per float.

/// The length scaled by a bare number.
impl<S: Space<Repr = f32>> Mul<f32> for Px<S> {
    type Output = Self;
    fn mul(self, factor: f32) -> Self {
        Self::new(self.0 * factor)
    }
}

/// The length divided by a bare number.
impl<S: Space<Repr = f32>> Div<f32> for Px<S> {
    type Output = Self;
    fn div(self, divisor: f32) -> Self {
        Self::new(self.0 / divisor)
    }
}

/// The length scaled by a bare number.
impl<S: Space<Repr = f64>> Mul<f64> for Px<S> {
    type Output = Self;
    fn mul(self, factor: f64) -> Self {
        Self::new(self.0 * factor)
    }
}

/// The length divided by a bare number.
impl<S: Space<Repr = f64>> Div<f64> for Px<S> {
    type Output = Self;
    fn div(self, divisor: f64) -> Self {
        Self::new(self.0 / divisor)
    }
}

/// The ratio of two lengths, a bare number.
impl<S: Space> Div for Px<S>
where
    S::Repr: Float,
{
    type Output = S::Repr;
    fn div(self, other: Self) -> S::Repr {
        self.0 / other.0
    }
}

/// The length carried into the space `B` by `scale`.
impl<A: ScalesTo<B>, B: Space> Mul<Scale<A, B>> for Px<A>
where
    A::Repr: Rescale<B::Repr>,
{
    type Output = Px<B>;
    fn mul(self, scale: Scale<A, B>) -> Px<B> {
        Px::new(self.0.scaled(scale.get()))
    }
}

/// The length carried back into the space `A`, before `scale`.
impl<A: ScalesTo<B>, B: Space> Div<Scale<A, B>> for Px<B>
where
    B::Repr: Rescale<A::Repr>,
{
    type Output = Px<A>;
    fn div(self, scale: Scale<A, B>) -> Px<A> {
        Px::new(self.0.unscaled(scale.get()))
    }
}

impl<S: Space> std::iter::Sum for Px<S> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self::new(iter.map(|length| length.0).sum())
    }
}

mod sealed {
    /// Closes [`super::PrintUnit`] to the two units of print this module
    /// declares.
    pub trait Sealed {}

    impl Sealed for super::Pt {}
    impl Sealed for super::Mm {}
}

/// A unit of print length: the marker a [`PrintLength`] names, and the
/// fixed number of document pixels in one of it.
///
/// The trait is sealed. A page measures in points and millimetres, and in
/// nothing else.
pub trait PrintUnit: sealed::Sealed + 'static {
    /// The document pixels in one unit: a CSS pixel is 1/96 inch.
    const DOC_PX: f32;

    /// The name of the unit, as `Debug` prints a length of it.
    const NAME: crate::StaticKey<crate::DebugNames>;
}

/// A typographic point, 1/72 inch: an indent, a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pt {}

impl PrintUnit for Pt {
    const DOC_PX: f32 = PX_PER_PT;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Pt");
}

/// A millimetre: a page size, a margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mm {}

impl PrintUnit for Mm {
    const DOC_PX: f32 = PX_PER_MM;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Mm");
}

/// A length of print, in the unit `U` names, as `PrintLength<Mm>`.
///
/// Two lengths of one unit add, subtract and divide into a bare ratio. A
/// length scales by a bare number. A product of two lengths does not
/// compile. A length of one unit meets a length of the other only through
/// [`PrintLength::in_unit`], and a pixel length only through a named method.
///
/// `new` wraps the number, and `get` unwraps it for a foreign API. No rule
/// bounds a length, so the type has no fallible constructor. The serde form
/// is the bare `f32`, as `#[serde(transparent)]` writes it.
pub struct PrintLength<U: PrintUnit>(f32, PhantomData<fn() -> U>);

impl<U: PrintUnit> PrintLength<U> {
    /// The length of zero.
    pub const ZERO: Self = Self(0.0, PhantomData);

    /// The length of `value` units.
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self(value, PhantomData)
    }

    /// The number of units, for a foreign API that takes the float.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }

    /// The same length in the unit `V`, through the document pixels both
    /// units are a fixed number of.
    #[must_use]
    pub const fn in_unit<V: PrintUnit>(self) -> PrintLength<V> {
        PrintLength::new(self.0 * U::DOC_PX / V::DOC_PX)
    }

    /// The length in document pixels.
    #[must_use]
    pub fn to_doc_px(self) -> Px<Doc> {
        Px::new(self.0 * U::DOC_PX)
    }

    /// The smaller of the two lengths.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        Self::new(self.0.min(other.0))
    }

    /// The larger of the two lengths.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self::new(self.0.max(other.0))
    }

    /// The length without its sign.
    #[must_use]
    pub fn abs(self) -> Self {
        Self::new(self.0.abs())
    }

    /// The length clamped into `lo ..= hi`, never a panic: a NaN reads as
    /// `lo`, and `hi` wins when the bounds cross.
    #[must_use]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        Self::new(self.0.max(lo.0).min(hi.0))
    }

    /// The length rounded to a whole number of units.
    #[must_use]
    pub fn round(self) -> Self {
        Self::new(self.0.round())
    }

    /// The length rounded down to a whole number of units.
    #[must_use]
    pub fn floor(self) -> Self {
        Self::new(self.0.floor())
    }

    /// Whether the length is a finite number.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

// Every impl below is written by hand, as for `Px`: a derive would bound
// the marker by a trait the marker has no use for.
impl<U: PrintUnit> Clone for PrintLength<U> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<U: PrintUnit> Copy for PrintLength<U> {}

impl<U: PrintUnit> Default for PrintLength<U> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<U: PrintUnit> PartialEq for PrintLength<U> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<U: PrintUnit> PartialOrd for PrintLength<U> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

/// The unit and the number, as `PrintLength<Mm>(210.0)`.
impl<U: PrintUnit> std::fmt::Debug for PrintLength<U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrintLength<{}>({:?})", U::NAME.as_str(), self.0)
    }
}

/// The bare number, as `#[serde(transparent)]` writes it.
impl<U: PrintUnit> Serialize for PrintLength<U> {
    fn serialize<W: Serializer>(&self, serializer: W) -> Result<W::Ok, W::Error> {
        self.0.serialize(serializer)
    }
}

/// The bare number, as `#[serde(transparent)]` reads it.
impl<'de, U: PrintUnit> Deserialize<'de> for PrintLength<U> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f32::deserialize(deserializer).map(Self::new)
    }
}

impl<U: PrintUnit> Add for PrintLength<U> {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.0 + other.0)
    }
}

impl<U: PrintUnit> Sub for PrintLength<U> {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.0 - other.0)
    }
}

impl<U: PrintUnit> AddAssign for PrintLength<U> {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl<U: PrintUnit> SubAssign for PrintLength<U> {
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl<U: PrintUnit> Neg for PrintLength<U> {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.0)
    }
}

/// The length scaled by a bare number.
impl<U: PrintUnit> Mul<f32> for PrintLength<U> {
    type Output = Self;
    fn mul(self, factor: f32) -> Self {
        Self::new(self.0 * factor)
    }
}

/// The length divided by a bare number.
impl<U: PrintUnit> Div<f32> for PrintLength<U> {
    type Output = Self;
    fn div(self, divisor: f32) -> Self {
        Self::new(self.0 / divisor)
    }
}

/// The ratio of two lengths, a bare number.
impl<U: PrintUnit> Div for PrintLength<U> {
    type Output = f32;
    fn div(self, other: Self) -> f32 {
        self.0 / other.0
    }
}

impl<U: PrintUnit> std::iter::Sum for PrintLength<U> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self::new(iter.map(|length| length.0).sum())
    }
}

/// An angle in radians, clockwise on a screen whose y axis points down.
///
/// No other unit of angle lives in the workspace: a writer whose format takes
/// degrees reads [`Radians::to_degrees`]. The serde form is the bare `f64`.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Radians(f64);

impl Radians {
    /// The angle of zero.
    pub const ZERO: Self = Self(0.0);

    /// The angle of `value` radians.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// The number of radians, for a foreign API that takes the float.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    /// The angle in degrees, for a writer whose format takes degrees.
    #[must_use]
    pub fn to_degrees(self) -> f64 {
        self.0.to_degrees()
    }

    /// Whether the angle is a finite number.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

impl Add for Radians {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Radians {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Neg for Radians {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// The angle scaled by a bare number.
impl Mul<f64> for Radians {
    type Output = Self;
    fn mul(self, factor: f64) -> Self {
        Self(self.0 * factor)
    }
}

/// A speed along one axis of the zoomed view, in view pixels per
/// millisecond: what a flick or a glide moves at.
///
/// [`Px::per`] writes one from a move and a span, and [`ViewSpeed::over`]
/// turns it back into a move. The serde form is the bare `f32`.
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ViewSpeed(f32);

impl ViewSpeed {
    /// The speed of zero.
    pub const ZERO: Self = Self(0.0);

    /// The speed of `value` view pixels per millisecond.
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// The number of view pixels per millisecond, for a foreign API that
    /// takes the float.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }

    /// The speed without its sign.
    #[must_use]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    /// The length a move at this speed covers over `span`.
    #[must_use]
    pub fn over(self, span: TimeSpan<FracMs>) -> Px<View> {
        Px::new(self.0 * span.get() as f32)
    }
}

impl Add for ViewSpeed {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for ViewSpeed {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Neg for ViewSpeed {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// The speed scaled by a bare number: a decay, a blend weight.
impl Mul<f32> for ViewSpeed {
    type Output = Self;
    fn mul(self, factor: f32) -> Self {
        Self(self.0 * factor)
    }
}

impl Px<StoredBoard> {
    /// The coordinate in the `f64` a board is measured in. Every `f32` has an
    /// exact `f64`, so nothing is lost.
    #[must_use]
    pub fn widened(self) -> Px<Board> {
        Px::new(f64::from(self.0))
    }
}

impl Px<Board> {
    /// The length on a drawing surface under the board camera's `zoom`,
    /// in the surface's `f64`, so a padding keeps its precision.
    #[must_use]
    pub fn zoomed_on_surface(self, zoom: Scale<Board, View>) -> Px<Surface> {
        Px::new(self.0 * zoom.get())
    }
}

impl Px<Doc> {
    /// The length on the view at `zoom`.
    #[must_use]
    pub fn zoomed(self, zoom: Scale<Doc, View>) -> Px<View> {
        self * zoom
    }

    /// The length on a drawing surface at `zoom`, scaled in the surface's
    /// `f64` so a sheet far down keeps its precision.
    #[must_use]
    pub fn zoomed_on_surface(self, zoom: Scale<Doc, View>) -> Px<Surface> {
        Px::new(self.0.scaled(zoom.get()))
    }

    /// The length in points.
    #[must_use]
    pub fn to_pt(self) -> PrintLength<Pt> {
        PrintLength::new(self.0 / Pt::DOC_PX)
    }
}

impl Px<View> {
    /// The length in the document, before `zoom`.
    #[must_use]
    pub fn unzoomed(self, zoom: Scale<Doc, View>) -> Px<Doc> {
        self / zoom
    }

    /// The coordinate on the page, for a canvas whose corner sits at
    /// `canvas_origin`. A view pixel is a CSS pixel in size, so only the
    /// origin moves.
    #[must_use]
    pub fn on_page(self, canvas_origin: Px<Css>) -> Px<Css> {
        Px::new(canvas_origin.0 + self.0)
    }

    /// The same length in CSS pixels. A view pixel is a CSS pixel in size, so
    /// a length carries over as it is. A coordinate goes through
    /// [`Self::on_page`] instead.
    #[must_use]
    pub const fn css_length(self) -> Px<Css> {
        Px::new(self.0)
    }

    /// The coordinate on the surface, for a view whose top-left corner sits at
    /// `view_origin` on it: the inverse of [`Px::in_view`].
    #[must_use]
    pub fn on_surface(self, view_origin: Px<Surface>) -> Px<Surface> {
        Px::new(view_origin.0 + f64::from(self.0))
    }

    /// The same length in surface pixels. A view pixel is a surface pixel in
    /// size, and every `f32` has an exact `f64`, so nothing is lost. A
    /// coordinate goes through [`Self::on_surface`] instead.
    #[must_use]
    pub const fn surface_length(self) -> Px<Surface> {
        Px::new(self.0 as f64)
    }

    /// The speed of a move of this length over `span`.
    #[must_use]
    pub fn per(self, span: TimeSpan<FracMs>) -> ViewSpeed {
        ViewSpeed(self.0 / span.get() as f32)
    }
}

impl Px<Css> {
    /// The coordinate on a canvas whose corner sits at `canvas_origin`: the
    /// inverse of [`Px::on_page`].
    #[must_use]
    pub fn in_canvas(self, canvas_origin: Px<Css>) -> Px<View> {
        Px::new(self.0 - canvas_origin.0)
    }

    /// The same length on the view, such as a wheel delta. A view pixel is a
    /// CSS pixel in size (`CANVAS-032`), so a length crosses one for one. A
    /// coordinate takes [`Px::in_canvas`] instead.
    #[must_use]
    pub const fn view_length(self) -> Px<View> {
        Px::new(self.0)
    }
}

impl Px<CanvasCss> {
    /// The same length in surface pixels. A CSS pixel is a surface pixel in
    /// size (`CANVAS-032`), and every `u32` has an exact `f64`, so nothing is
    /// lost.
    #[must_use]
    pub fn surface_length(self) -> Px<Surface> {
        Px::new(f64::from(self.0))
    }
}

impl Px<Surface> {
    /// The coordinate on the view, for a view whose top-left corner sits at
    /// `view_origin` on the surface.
    #[must_use]
    pub fn in_view(self, view_origin: Px<Surface>) -> Px<View> {
        Px::new((self.0 - view_origin.0) as f32)
    }

    /// The same length in view pixels. A surface pixel is a view pixel in size,
    /// so a length carries over as it is. A coordinate goes through
    /// [`Self::in_view`] instead.
    #[must_use]
    pub const fn view_length(self) -> Px<View> {
        Px::new(self.0 as f32)
    }
}

/// A point of one pixel space, over the length type `P` of that space, as
/// `Point<Px<View>>`.
///
/// Two points of one space subtract into the [`Vector`] between them, and a
/// point moves by a vector. Two points never add: the sum names no place.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point<P> {
    pub x: P,
    pub y: P,
}

impl<P> Point<P> {
    /// The point at `x`, `y`.
    #[must_use]
    pub const fn new(x: P, y: P) -> Self {
        Self { x, y }
    }
}

/// A displacement in one space: a wheel delta, a drag, a scroll step. It
/// holds the length type of its space, or a speed, as `Vector<ViewSpeed>`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vector<P> {
    pub dx: P,
    pub dy: P,
}

impl<P> Vector<P> {
    /// The displacement of `dx` across and `dy` down.
    #[must_use]
    pub const fn new(dx: P, dy: P) -> Self {
        Self { dx, dy }
    }
}

/// A size of one length space: a width and a height, kept apart by name so
/// the two never swap. A size of whole pixels keeps its space as a type, as
/// `Size<Px<Image>>` does. The size of an image then never reaches a place
/// that sizes a buffer or a box.
///
/// Its serde form is the two edges, width first. An edge of `Px` is the bare
/// number, so a size of whole pixels writes the bytes two `u32` fields wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size<P> {
    pub width: P,
    pub height: P,
}

impl<P> Size<P> {
    /// The size of `width` across and `height` down.
    #[must_use]
    pub const fn new(width: P, height: P) -> Self {
        Self { width, height }
    }
}

/// The vector from `other` to `self`.
impl<P: Sub<Output = P>> Sub for Point<P> {
    type Output = Vector<P>;
    fn sub(self, other: Self) -> Vector<P> {
        Vector::new(self.x - other.x, self.y - other.y)
    }
}

/// The point moved by `by`.
impl<P: Add<Output = P>> Add<Vector<P>> for Point<P> {
    type Output = Self;
    fn add(self, by: Vector<P>) -> Self {
        Point::new(self.x + by.dx, self.y + by.dy)
    }
}

/// The point moved back by `by`.
impl<P: Sub<Output = P>> Sub<Vector<P>> for Point<P> {
    type Output = Self;
    fn sub(self, by: Vector<P>) -> Self {
        Point::new(self.x - by.dx, self.y - by.dy)
    }
}

/// The two displacements one after the other.
impl<P: Add<Output = P>> Add for Vector<P> {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Vector::new(self.dx + other.dx, self.dy + other.dy)
    }
}

/// A velocity on the zoomed view, in view pixels per millisecond on each
/// axis: what a flick leaves.
impl Vector<ViewSpeed> {
    /// The speed along the direction of travel.
    #[must_use]
    pub fn magnitude(self) -> ViewSpeed {
        ViewSpeed(self.dx.0.hypot(self.dy.0))
    }

    /// The velocity scaled by a bare number: a decay, a blend weight.
    #[must_use]
    pub fn scaled(self, factor: f32) -> Self {
        Vector::new(self.dx * factor, self.dy * factor)
    }
}

impl Vector<Px<View>> {
    /// The length of the displacement.
    #[must_use]
    pub fn length(self) -> Px<View> {
        Px::new(self.dx.0.hypot(self.dy.0))
    }

    /// The velocity of this displacement made over `span`.
    #[must_use]
    pub fn per(self, span: TimeSpan<FracMs>) -> Vector<ViewSpeed> {
        Vector::new(self.dx.per(span), self.dy.per(span))
    }
}

impl Point<Px<View>> {
    /// The point on the page, for a canvas whose corner sits at
    /// `canvas_origin`.
    #[must_use]
    pub fn on_page(self, canvas_origin: Point<Px<Css>>) -> Point<Px<Css>> {
        Point::new(
            self.x.on_page(canvas_origin.x),
            self.y.on_page(canvas_origin.y),
        )
    }
}

impl Point<Px<Css>> {
    /// The point on a canvas whose corner sits at `canvas_origin`.
    #[must_use]
    pub fn in_canvas(self, canvas_origin: Point<Px<Css>>) -> Point<Px<View>> {
        Point::new(
            self.x.in_canvas(canvas_origin.x),
            self.y.in_canvas(canvas_origin.y),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Mm, PrintLength, Pt, ViewSpeed};
    use crate::{CanvasCss, Css, Doc, Image, Point, Px, Scale, Size, Surface, Vector, View};
    use crate::{FracMs, TimeSpan};

    #[test]
    fn a_document_length_scales_by_the_zoom_and_back() {
        let zoom = Scale::<Doc, View>::positive(2.0);
        assert_eq!(Px::<Doc>::new(12.0).zoomed(zoom), Px::<View>::new(24.0));
        assert_eq!(Px::<View>::new(24.0).unzoomed(zoom), Px::<Doc>::new(12.0));
        assert_eq!(
            Px::<Doc>::new(12.0).zoomed_on_surface(zoom),
            Px::<Surface>::new(24.0)
        );
    }

    #[test]
    fn a_point_is_four_thirds_of_a_pixel_and_a_millimetre_is_a_ninety_sixth_of_an_inch() {
        assert_eq!(
            PrintLength::<Pt>::new(12.0).to_doc_px(),
            Px::<Doc>::new(16.0)
        );
        assert_eq!(Px::<Doc>::new(16.0).to_pt(), PrintLength::<Pt>::new(12.0));
        assert!((PrintLength::<Mm>::new(25.4).to_doc_px().get() - 96.0).abs() < 1e-4);
        assert!((PrintLength::<Mm>::new(25.4).in_unit::<Pt>().get() - 72.0).abs() < 1e-4);
    }

    #[test]
    fn a_length_adds_scales_and_divides_into_a_ratio() {
        let a = Px::<View>::new(3.0);
        assert_eq!(a + a - Px::<View>::new(1.0), Px::<View>::new(5.0));
        assert_eq!(a * 2.0, Px::<View>::new(6.0));
        assert_eq!(Px::<View>::new(6.0) / a, 2.0);
        assert_eq!(-a, Px::<View>::new(-3.0));
        assert_eq!([a, a].into_iter().sum::<Px<View>>(), Px::<View>::new(6.0));
    }

    #[test]
    fn a_canvas_box_edge_carries_over_to_the_surface_as_it_is() {
        assert_eq!(
            Px::<CanvasCss>::new(u32::MAX).surface_length(),
            Px::<Surface>::new(f64::from(u32::MAX))
        );
    }

    #[test]
    fn a_view_point_moves_onto_the_page_by_the_canvas_origin() {
        let origin = Point::new(Px::<Css>::new(10.0), Px::<Css>::new(20.0));
        let on_page = Point::new(Px::<View>::new(1.0), Px::<View>::new(2.0)).on_page(origin);
        assert_eq!(
            on_page,
            Point::new(Px::<Css>::new(11.0), Px::<Css>::new(22.0))
        );
        assert_eq!(
            on_page.in_canvas(origin),
            Point::new(Px::<View>::new(1.0), Px::<View>::new(2.0))
        );
    }

    #[test]
    fn two_points_subtract_into_a_vector_that_moves_one_onto_the_other() {
        let from = Point::new(Px::<View>::new(1.0), Px::<View>::new(2.0));
        let to = Point::new(Px::<View>::new(4.0), Px::<View>::new(-2.0));
        let by = to - from;
        assert_eq!(by, Vector::new(Px::<View>::new(3.0), Px::<View>::new(-4.0)));
        assert_eq!(from + by, to);
        assert_eq!(to - by, from);
        assert_eq!(
            by + by,
            Vector::new(Px::<View>::new(6.0), Px::<View>::new(-8.0))
        );
    }

    /// A sheet position past the reach of an `f32` keeps its fraction until it
    /// is measured from the view's corner.
    #[test]
    fn a_surface_position_keeps_its_precision_until_it_reaches_the_view() {
        let origin = Px::<Surface>::new(2_000_000.0);
        let at = Px::<Surface>::new(2_000_010.25);
        assert_eq!(at.in_view(origin), Px::<View>::new(10.25));
        assert_eq!(Px::<View>::new(10.25).on_surface(origin), at);
    }

    #[test]
    fn a_move_over_a_span_is_a_speed_that_covers_the_move_again() {
        let span = TimeSpan::<FracMs>::new(4.0);
        let by = Vector::new(Px::<View>::new(3.0), Px::<View>::new(4.0));
        let velocity = by.per(span);
        assert_eq!(velocity.dy, ViewSpeed::new(1.0));
        assert_eq!(velocity.magnitude(), ViewSpeed::new(1.25));
        assert_eq!(velocity.dx.over(span), Px::<View>::new(3.0));
        assert_eq!(by.length(), Px::<View>::new(5.0));
        assert_eq!(velocity.scaled(2.0).dy, ViewSpeed::new(2.0));
    }

    #[test]
    fn a_length_serializes_as_its_bare_float() {
        assert_eq!(
            serde_json::to_string(&PrintLength::<Mm>::new(210.0)).unwrap(),
            "210.0"
        );
    }

    #[test]
    fn a_pixel_size_keeps_the_bytes_of_its_two_bare_edges() {
        // The bytes two `u32` fields wrote, before each edge was a `Px`.
        let size = Size::new(Px::new(640), Px::new(480));
        assert_eq!(
            serde_json::to_string(&size).unwrap(),
            r#"{"width":640,"height":480}"#
        );
        let bytes = postcard::to_allocvec(&size).unwrap();
        assert_eq!(bytes, [0x80, 0x05, 0xE0, 0x03]);
        assert_eq!(
            postcard::from_bytes::<Size<Px<Image>>>(&bytes).unwrap(),
            size
        );
    }
}
