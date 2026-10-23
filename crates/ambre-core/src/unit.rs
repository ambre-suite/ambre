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
//! | [`DocPx`] | a pixel of the document, before the zoom | a column width, a run offset |
//! | [`ViewPx`] | a pixel of the zoomed view, from the canvas's top-left corner | a pointer, a caret |
//! | [`CssPx`] | a CSS pixel of the page, outside any canvas | a wheel delta, a fixed element |
//! | [`Pt`] | a typographic point, 1/72 inch | an indent, a table column |
//! | [`Mm`] | a millimetre | a page size, a margin |
//! | [`Radians`] | an angle | a rotated element |
//!
//! No type converts into another through `From`. Each conversion is a named
//! method that takes the factor it needs. [`DocPx::zoomed`] takes the zoom,
//! and [`Pt::to_doc_px`] applies the fixed ratio of the two units. A
//! device pixel is the drawing surface's concern, so `ambre-canvas` holds that
//! type and the density (`CANVAS-031`).

use serde::{Deserialize, Serialize};

use crate::font::PX_PER_PT;
use crate::zoom::ZoomFactor;

/// CSS pixels per millimetre: a CSS pixel is 1/96 inch, an inch is 25.4 mm.
pub const PX_PER_MM: f32 = 96.0 / 25.4;

/// Writes one length type over a float.
///
/// The operators are the ones a length allows. Two lengths add, subtract
/// and divide into a bare ratio. A length negates, and scales by a bare
/// number. A product of two lengths does not compile.
///
/// `new` wraps the number, and `get` unwraps it for a foreign API.
/// No rule bounds a length, so the type has no fallible constructor. The
/// serde form is the bare float, through `#[serde(transparent)]`.
macro_rules! define_length {
    ($(#[$meta:meta])* $name:ident($inner:ty)) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name($inner);

        impl $name {
            /// The length of zero.
            pub const ZERO: Self = Self(0.0);

            /// The length of `value` units.
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// The number of units, for a foreign API that takes the float.
            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }

            /// The smaller of the two lengths.
            #[must_use]
            pub fn min(self, other: Self) -> Self {
                Self(self.0.min(other.0))
            }

            /// The larger of the two lengths.
            #[must_use]
            pub fn max(self, other: Self) -> Self {
                Self(self.0.max(other.0))
            }

            /// The length without its sign.
            #[must_use]
            pub fn abs(self) -> Self {
                Self(self.0.abs())
            }

            /// The length clamped into `lo ..= hi`, never a panic: a NaN reads
            /// as `lo`, and `hi` wins when the bounds cross.
            #[must_use]
            pub fn clamp(self, lo: Self, hi: Self) -> Self {
                Self(self.0.max(lo.0).min(hi.0))
            }

            /// The length rounded to a whole number of units.
            #[must_use]
            pub fn round(self) -> Self {
                Self(self.0.round())
            }

            /// The length rounded down to a whole number of units.
            #[must_use]
            pub fn floor(self) -> Self {
                Self(self.0.floor())
            }

            /// Whether the length is a finite number.
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

        /// The length scaled by a bare number.
        impl std::ops::Mul<$inner> for $name {
            type Output = Self;
            fn mul(self, factor: $inner) -> Self {
                Self(self.0 * factor)
            }
        }

        /// The length divided by a bare number.
        impl std::ops::Div<$inner> for $name {
            type Output = Self;
            fn div(self, divisor: $inner) -> Self {
                Self(self.0 / divisor)
            }
        }

        /// The ratio of two lengths, a bare number.
        impl std::ops::Div for $name {
            type Output = $inner;
            fn div(self, other: Self) -> $inner {
                self.0 / other.0
            }
        }

        impl std::iter::Sum for $name {
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                Self(iter.map(|length| length.0).sum())
            }
        }
    };
}

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

impl DocPx {
    /// The length on the view at `zoom`.
    #[must_use]
    pub fn zoomed(self, zoom: ZoomFactor) -> ViewPx {
        ViewPx(self.0 * zoom.get() as f32)
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

/// A point of one pixel space: [`ViewPoint`] or [`CssPoint`].
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

/// A point of the zoomed view, from the canvas's top-left corner.
pub type ViewPoint = Point<ViewPx>;

/// A point of the page, in CSS pixels.
pub type CssPoint = Point<CssPx>;

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

#[cfg(test)]
mod tests {
    use super::{CssPoint, CssPx, DocPx, Mm, Pt, ViewPoint, ViewPx};
    use crate::ZoomFactor;

    #[test]
    fn a_document_length_scales_by_the_zoom_and_back() {
        let zoom = ZoomFactor::positive(2.0);
        assert_eq!(DocPx::new(12.0).zoomed(zoom), ViewPx::new(24.0));
        assert_eq!(ViewPx::new(24.0).unzoomed(zoom), DocPx::new(12.0));
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
    }

    #[test]
    fn a_length_serializes_as_its_bare_float() {
        assert_eq!(serde_json::to_string(&Mm::new(210.0)).unwrap(), "210.0");
    }
}
