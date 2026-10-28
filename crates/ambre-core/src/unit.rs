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
//! | [`DocPx`] | a document pixel, before the zoom | a column width, a run offset |
//! | [`ViewPx`] | a pixel of the zoomed view, from the canvas corner | a pointer, a caret |
//! | [`CssPx`] | a CSS pixel of the page, outside any canvas | a wheel delta, a fixed element |
//! | [`SurfacePx`] | a drawing-surface pixel, as an `f64` | an absolute sheet position |
//! | [`Pt`] | a typographic point, 1/72 inch | an indent, a table column |
//! | [`Mm`] | a millimetre | a page size, a margin |
//! | [`Radians`] | an angle | a rotated element |
//! | [`ImageSize`] | the whole pixels of a decoded image | a picture preview |
//! | [`ViewSpeed`] | view pixels per millisecond | a flick, a glide |
//! | [`BoardLength`] | a board world unit, before the zoom | a frame's padding, a hit reach |
//! | [`StrokeCoord`] | a board world unit, as the `f32` a stroke stores | a freehand sample |
//!
//! No type converts into another through `From`. Each conversion is a named
//! method that takes the factor it needs. [`DocPx::zoomed`] takes the zoom,
//! and [`Pt::to_doc_px`] applies the fixed ratio of the two units. A
//! device pixel is the drawing surface's concern, so `ambre-canvas` holds that
//! type and the density (`CANVAS-031`).

use serde::{Deserialize, Serialize};

use crate::font::PX_PER_PT;
use crate::span::PageMillis;
use crate::zoom::ZoomFactor;

/// CSS pixels per millimetre: a CSS pixel is 1/96 inch, an inch is 25.4 mm.
pub const PX_PER_MM: f32 = 96.0 / 25.4;

/// Writes one measured quantity type over a float: a length, an angle, or a
/// span of the page clock (`crate::span::PageMillis`).
///
/// The operators are the ones a quantity allows. Two quantities add,
/// subtract and divide into a bare ratio. A quantity negates, and scales by
/// a bare number. A product of two quantities does not compile.
///
/// `new` wraps the number, and `get` unwraps it for a foreign API.
/// No rule bounds a quantity, so the type has no fallible constructor. The
/// serde form is the bare float, through `#[serde(transparent)]`.
macro_rules! define_length {
    ($(#[$meta:meta])* $name:ident($inner:ty)) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name($inner);

        impl $name {
            /// The quantity of zero.
            pub const ZERO: Self = Self(0.0);

            /// The quantity of `value` units.
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// The number of units, for a foreign API that takes the float.
            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }

            /// The smaller of the two quantities.
            #[must_use]
            pub fn min(self, other: Self) -> Self {
                Self(self.0.min(other.0))
            }

            /// The larger of the two quantities.
            #[must_use]
            pub fn max(self, other: Self) -> Self {
                Self(self.0.max(other.0))
            }

            /// The quantity without its sign.
            #[must_use]
            pub fn abs(self) -> Self {
                Self(self.0.abs())
            }

            /// The quantity clamped into `lo ..= hi`, never a panic: a NaN reads
            /// as `lo`, and `hi` wins when the bounds cross.
            #[must_use]
            pub fn clamp(self, lo: Self, hi: Self) -> Self {
                Self(self.0.max(lo.0).min(hi.0))
            }

            /// The quantity rounded to a whole number of units.
            #[must_use]
            pub fn round(self) -> Self {
                Self(self.0.round())
            }

            /// The quantity rounded down to a whole number of units.
            #[must_use]
            pub fn floor(self) -> Self {
                Self(self.0.floor())
            }

            /// Whether the quantity is a finite number.
            #[must_use]
            pub fn is_finite(self) -> bool {
                self.0.is_finite()
            }
        }

        impl std::ops::Add for $name {
            type Output = Self;
            fn add(self, other: Self) -> Self {
                Self(self.0 + other.0)
            }
        }

        impl std::ops::Sub for $name {
            type Output = Self;
            fn sub(self, other: Self) -> Self {
                Self(self.0 - other.0)
            }
        }

        impl std::ops::AddAssign for $name {
            fn add_assign(&mut self, other: Self) {
                self.0 += other.0;
            }
        }

        impl std::ops::SubAssign for $name {
            fn sub_assign(&mut self, other: Self) {
                self.0 -= other.0;
            }
        }

        impl std::ops::Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }

        /// The quantity scaled by a bare number.
        impl std::ops::Mul<$inner> for $name {
            type Output = Self;
            fn mul(self, factor: $inner) -> Self {
                Self(self.0 * factor)
            }
        }

        /// The quantity divided by a bare number.
        impl std::ops::Div<$inner> for $name {
            type Output = Self;
            fn div(self, divisor: $inner) -> Self {
                Self(self.0 / divisor)
            }
        }

        /// The ratio of two quantities, a bare number.
        impl std::ops::Div for $name {
            type Output = $inner;
            fn div(self, other: Self) -> $inner {
                self.0 / other.0
            }
        }

        impl std::iter::Sum for $name {
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                Self(iter.map(|quantity| quantity.0).sum())
            }
        }
    };
}

pub(crate) use define_length;

define_length! {
    /// A length in pixels of the document, before the zoom: what a layout
    /// measures a page, a column or a run in.
    DocPx(f32)
}

define_length! {
    /// A length in pixels of the zoomed view, and a coordinate measured from
    /// the top-left corner of the canvas.
    ///
    /// A view pixel is a CSS pixel in size (`CANVAS-032`). Only the origin
    /// differs from [`CssPx`]: the canvas's corner, not the page's.
    ViewPx(f32)
}

define_length! {
    /// A length in CSS pixels of the page, outside any canvas: a wheel delta,
    /// or the place of an element fixed on the page.
    CssPx(f32)
}

define_length! {
    /// A coordinate or a length on a drawing surface, in the `f64` the
    /// surface takes.
    ///
    /// A sheet walks its cells at absolute positions, at zoom 1: a hundred
    /// thousand rows of twenty pixels reach two million pixels. An `f32`
    /// steps by a quarter pixel there, so the three `f32` spaces above cannot
    /// hold such a position. [`SurfacePx::in_view`] turns one into a
    /// [`ViewPx`] once it is measured from the corner of the view.
    SurfacePx(f64)
}

define_length! {
    /// A length in typographic points, 1/72 inch.
    Pt(f32)
}

define_length! {
    /// A length in millimetres.
    Mm(f32)
}

define_length! {
    /// An angle in radians, clockwise on a screen whose y axis points down.
    Radians(f64)
}

define_length! {
    /// A speed along one axis of the zoomed view, in view pixels per
    /// millisecond: what a flick or a glide moves at.
    ViewSpeed(f32)
}

define_length! {
    /// A length on a board, in world units: what a board's camera scales by
    /// the zoom into view pixels.
    ///
    /// A view length and a board length are both floats. A reach of four view
    /// pixels taken as four board units picks wrong at every zoom but 1. The
    /// camera's named conversion stands between the two. The `f64` is the one
    /// kurbo measures a board in.
    BoardLength(f64)
}

define_length! {
    /// A coordinate of a freehand sample on a board, in world units, held in
    /// the `f32` a stroke stores so a sample stays twelve bytes.
    /// [`StrokeCoord::widened`] reads it as the [`BoardLength`] the board
    /// computes in.
    StrokeCoord(f32)
}

impl StrokeCoord {
    /// The coordinate in the `f64` a board is measured in. Every `f32` has an
    /// exact `f64`, so nothing is lost.
    #[must_use]
    pub fn widened(self) -> BoardLength {
        BoardLength(f64::from(self.0))
    }
}

impl DocPx {
    /// The length on the view at `zoom`.
    #[must_use]
    pub fn zoomed(self, zoom: ZoomFactor) -> ViewPx {
        ViewPx(self.0 * zoom.get() as f32)
    }

    /// The length on a drawing surface at `zoom`, scaled in the surface's
    /// `f64` so a sheet far down keeps its precision.
    #[must_use]
    pub fn zoomed_on_surface(self, zoom: ZoomFactor) -> SurfacePx {
        SurfacePx(f64::from(self.0) * zoom.get())
    }

    /// The length in points.
    #[must_use]
    pub fn to_pt(self) -> Pt {
        Pt(self.0 / PX_PER_PT)
    }
}

impl ViewPx {
    /// The length in the document, before `zoom`.
    #[must_use]
    pub fn unzoomed(self, zoom: ZoomFactor) -> DocPx {
        DocPx(self.0 / zoom.get() as f32)
    }

    /// The coordinate on the page, for a canvas whose corner sits at
    /// `canvas_origin`. A view pixel is a CSS pixel in size, so only the
    /// origin moves.
    #[must_use]
    pub fn on_page(self, canvas_origin: CssPx) -> CssPx {
        CssPx(canvas_origin.0 + self.0)
    }
}

impl CssPx {
    /// The coordinate on a canvas whose corner sits at `canvas_origin`: the
    /// inverse of [`ViewPx::on_page`].
    #[must_use]
    pub fn in_canvas(self, canvas_origin: CssPx) -> ViewPx {
        ViewPx(self.0 - canvas_origin.0)
    }
}

impl SurfacePx {
    /// The coordinate on the view, for a view whose top-left corner sits at
    /// `view_origin` on the surface.
    #[must_use]
    pub fn in_view(self, view_origin: SurfacePx) -> ViewPx {
        ViewPx((self.0 - view_origin.0) as f32)
    }
}

impl ViewPx {
    /// The coordinate on the surface, for a view whose top-left corner sits at
    /// `view_origin` on it: the inverse of [`SurfacePx::in_view`].
    #[must_use]
    pub fn on_surface(self, view_origin: SurfacePx) -> SurfacePx {
        SurfacePx(view_origin.0 + f64::from(self.0))
    }
}

impl ViewPx {
    /// The speed of a move of this length over `span`.
    #[must_use]
    pub fn per(self, span: PageMillis) -> ViewSpeed {
        ViewSpeed(self.0 / span.get() as f32)
    }
}

impl ViewSpeed {
    /// The length a move at this speed covers over `span`.
    #[must_use]
    pub fn over(self, span: PageMillis) -> ViewPx {
        ViewPx(self.0 * span.get() as f32)
    }
}

impl Radians {
    /// The angle in degrees, for a writer whose format takes degrees.
    #[must_use]
    pub fn to_degrees(self) -> f64 {
        self.0.to_degrees()
    }
}

impl Pt {
    /// The length in document pixels.
    #[must_use]
    pub fn to_doc_px(self) -> DocPx {
        DocPx(self.0 * PX_PER_PT)
    }
}

impl Mm {
    /// The length in document pixels.
    #[must_use]
    pub fn to_doc_px(self) -> DocPx {
        DocPx(self.0 * PX_PER_MM)
    }
}

/// A point of one pixel space: [`DocPoint`], [`ViewPoint`] or [`CssPoint`].
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

/// A displacement in one pixel space: a wheel delta, a drag, a scroll step.
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
/// the two never swap.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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

/// A size of whole pixels, in the pixel space `P` names: an [`ImageSize`],
/// or the CSS and device sizes of `ambre-canvas`. The space is a type, so
/// the size of an image never reaches a place that sizes a buffer or a box.
///
/// Its serde form is the two edges, width first, as two `u32` fields were:
/// each edge is a [`WholePx`], transparent to serde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct PixelSize<P> {
    pub width: WholePx<P>,
    pub height: WholePx<P>,
}

impl<P> PixelSize<P> {
    /// The size of `width` x `height` pixels.
    #[must_use]
    pub const fn new(width: WholePx<P>, height: WholePx<P>) -> Self {
        Self { width, height }
    }
}

/// One edge of a [`PixelSize`]: a whole number of pixels in the space `P`
/// names. The `u32` is what wgpu, the canvas element and the image decoders
/// take, so [`WholePx::get`] is the one way out to them.
///
/// Transparent to serde: it carries the bare `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct WholePx<P>(u32, #[serde(skip)] std::marker::PhantomData<P>);

impl<P> WholePx<P> {
    /// No pixel.
    pub const ZERO: Self = Self::new(0);

    /// An edge of `n` pixels.
    #[must_use]
    pub const fn new(n: u32) -> Self {
        Self(n, std::marker::PhantomData)
    }

    /// The bare count, for the call into a foreign API.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The bare count, as an error message prints a dimension.
impl<P> std::fmt::Display for WholePx<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// By hand: a derive would ask the space marker for an order it has no use for.
impl<P: Eq> PartialOrd for WholePx<P> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: Eq> Ord for WholePx<P> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

/// A pixel of a decoded image: one sample of its bitmap, never a length on a
/// screen. It names the space of [`ImageSize`] and holds no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImagePx {}

/// The size of a decoded image, in its own pixels.
pub type ImageSize = PixelSize<ImagePx>;

/// The vector from `other` to `self`.
impl<P: std::ops::Sub<Output = P>> std::ops::Sub for Point<P> {
    type Output = Vector<P>;
    fn sub(self, other: Self) -> Vector<P> {
        Vector::new(self.x - other.x, self.y - other.y)
    }
}

/// The point moved by `by`.
impl<P: std::ops::Add<Output = P>> std::ops::Add<Vector<P>> for Point<P> {
    type Output = Self;
    fn add(self, by: Vector<P>) -> Self {
        Point::new(self.x + by.dx, self.y + by.dy)
    }
}

/// The point moved back by `by`.
impl<P: std::ops::Sub<Output = P>> std::ops::Sub<Vector<P>> for Point<P> {
    type Output = Self;
    fn sub(self, by: Vector<P>) -> Self {
        Point::new(self.x - by.dx, self.y - by.dy)
    }
}

/// The two displacements one after the other.
impl<P: std::ops::Add<Output = P>> std::ops::Add for Vector<P> {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Vector::new(self.dx + other.dx, self.dy + other.dy)
    }
}

/// A point of the document, before the zoom, from the origin of the layout
/// that measures it: a page, or a run.
pub type DocPoint = Point<DocPx>;

/// A point of the zoomed view, from the canvas's top-left corner.
pub type ViewPoint = Point<ViewPx>;

/// A point of the page, in CSS pixels.
pub type CssPoint = Point<CssPx>;

/// A displacement of the page, in CSS pixels: a wheel delta.
pub type CssVector = Vector<CssPx>;

/// A displacement of the zoomed view.
pub type ViewVector = Vector<ViewPx>;

/// A size on the zoomed view: a canvas, a list, a box.
pub type ViewSize = Size<ViewPx>;

/// A velocity on the zoomed view, in view pixels per millisecond on each
/// axis: what a flick leaves.
pub type ViewSpeedVector = Vector<ViewSpeed>;

impl ViewSpeedVector {
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

impl ViewVector {
    /// The length of the displacement.
    #[must_use]
    pub fn length(self) -> ViewPx {
        ViewPx(self.dx.0.hypot(self.dy.0))
    }

    /// The velocity of this displacement made over `span`.
    #[must_use]
    pub fn per(self, span: PageMillis) -> ViewSpeedVector {
        Vector::new(self.dx.per(span), self.dy.per(span))
    }
}

impl ViewPoint {
    /// The point on the page, for a canvas whose corner sits at
    /// `canvas_origin`.
    #[must_use]
    pub fn on_page(self, canvas_origin: CssPoint) -> CssPoint {
        Point::new(
            self.x.on_page(canvas_origin.x),
            self.y.on_page(canvas_origin.y),
        )
    }
}

impl CssPoint {
    /// The point on a canvas whose corner sits at `canvas_origin`.
    #[must_use]
    pub fn in_canvas(self, canvas_origin: CssPoint) -> ViewPoint {
        Point::new(
            self.x.in_canvas(canvas_origin.x),
            self.y.in_canvas(canvas_origin.y),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CssPoint, CssPx, DocPx, ImageSize, Mm, Pt, SurfacePx, ViewPoint, ViewPx, ViewSpeed,
        ViewVector, WholePx,
    };
    use crate::{PageMillis, ZoomFactor};

    #[test]
    fn a_document_length_scales_by_the_zoom_and_back() {
        let zoom = ZoomFactor::positive(2.0);
        assert_eq!(DocPx::new(12.0).zoomed(zoom), ViewPx::new(24.0));
        assert_eq!(ViewPx::new(24.0).unzoomed(zoom), DocPx::new(12.0));
        assert_eq!(
            DocPx::new(12.0).zoomed_on_surface(zoom),
            SurfacePx::new(24.0)
        );
    }

    #[test]
    fn a_point_is_four_thirds_of_a_pixel_and_a_millimetre_is_a_ninety_sixth_of_an_inch() {
        assert_eq!(Pt::new(12.0).to_doc_px(), DocPx::new(16.0));
        assert_eq!(DocPx::new(16.0).to_pt(), Pt::new(12.0));
        assert!((Mm::new(25.4).to_doc_px().get() - 96.0).abs() < 1e-4);
    }

    #[test]
    fn a_length_adds_scales_and_divides_into_a_ratio() {
        let a = ViewPx::new(3.0);
        assert_eq!(a + a - ViewPx::new(1.0), ViewPx::new(5.0));
        assert_eq!(a * 2.0, ViewPx::new(6.0));
        assert_eq!(ViewPx::new(6.0) / a, 2.0);
        assert_eq!(-a, ViewPx::new(-3.0));
        assert_eq!([a, a].into_iter().sum::<ViewPx>(), ViewPx::new(6.0));
    }

    #[test]
    fn a_view_point_moves_onto_the_page_by_the_canvas_origin() {
        let origin = CssPoint::new(CssPx::new(10.0), CssPx::new(20.0));
        let on_page = ViewPoint::new(ViewPx::new(1.0), ViewPx::new(2.0)).on_page(origin);
        assert_eq!(on_page, CssPoint::new(CssPx::new(11.0), CssPx::new(22.0)));
        assert_eq!(
            on_page.in_canvas(origin),
            ViewPoint::new(ViewPx::new(1.0), ViewPx::new(2.0))
        );
    }

    #[test]
    fn two_points_subtract_into_a_vector_that_moves_one_onto_the_other() {
        let from = ViewPoint::new(ViewPx::new(1.0), ViewPx::new(2.0));
        let to = ViewPoint::new(ViewPx::new(4.0), ViewPx::new(-2.0));
        let by = to - from;
        assert_eq!(by, ViewVector::new(ViewPx::new(3.0), ViewPx::new(-4.0)));
        assert_eq!(from + by, to);
        assert_eq!(to - by, from);
        assert_eq!(
            by + by,
            ViewVector::new(ViewPx::new(6.0), ViewPx::new(-8.0))
        );
    }

    /// A sheet position past the reach of an `f32` keeps its fraction until it
    /// is measured from the view's corner.
    #[test]
    fn a_surface_position_keeps_its_precision_until_it_reaches_the_view() {
        let origin = SurfacePx::new(2_000_000.0);
        let at = SurfacePx::new(2_000_010.25);
        assert_eq!(at.in_view(origin), ViewPx::new(10.25));
        assert_eq!(ViewPx::new(10.25).on_surface(origin), at);
    }

    #[test]
    fn a_move_over_a_span_is_a_speed_that_covers_the_move_again() {
        let span = PageMillis::new(4.0);
        let by = ViewVector::new(ViewPx::new(3.0), ViewPx::new(4.0));
        let velocity = by.per(span);
        assert_eq!(velocity.dy, ViewSpeed::new(1.0));
        assert_eq!(velocity.magnitude(), ViewSpeed::new(1.25));
        assert_eq!(velocity.dx.over(span), ViewPx::new(3.0));
        assert_eq!(by.length(), ViewPx::new(5.0));
        assert_eq!(velocity.scaled(2.0).dy, ViewSpeed::new(2.0));
    }

    #[test]
    fn a_length_serializes_as_its_bare_float() {
        assert_eq!(serde_json::to_string(&Mm::new(210.0)).unwrap(), "210.0");
    }

    #[test]
    fn a_pixel_size_keeps_the_bytes_of_its_two_bare_edges() {
        // The bytes two `u32` fields wrote, before each edge was a `WholePx`.
        let size = ImageSize::new(WholePx::new(640), WholePx::new(480));
        assert_eq!(
            serde_json::to_string(&size).unwrap(),
            r#"{"width":640,"height":480}"#
        );
        let bytes = postcard::to_allocvec(&size).unwrap();
        assert_eq!(bytes, [0x80, 0x05, 0xE0, 0x03]);
        assert_eq!(postcard::from_bytes::<ImageSize>(&bytes).unwrap(), size);
    }
}
