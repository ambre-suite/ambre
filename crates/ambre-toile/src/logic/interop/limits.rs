//! Import safety limits (`toile-import-export` "Import trust").
//!
//! A foreign file is untrusted input: a hostile or merely huge SVG/raster must
//! not exhaust client memory or the R-tree. Every importer threads a [`Limits`]
//! and, on exceeding any bound, aborts the **whole** import atomically -- nothing
//! is applied (invariant). Coordinates are clamped rather than rejected so a
//! merely large (not malicious) drawing still imports, just bounded to the world.

use ambre_core::Px;
use ambre_core::{Board, Count, Elements};
use kurbo::{Point, Rect};

use super::ImportError;

/// How deep an import has walked into nested groups and frames: the root
/// is depth zero.
pub enum ImportDepth {}

impl ambre_core::CountKind for ImportDepth {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ImportDepth");
}

/// The symmetric bound an imported coordinate is clamped to, per axis. It is
/// finite and not negative, so the range `-bound..=bound` it clamps to is
/// never empty and never NaN.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordBound(Px<Board>);

impl CoordBound {
    /// `None` unless `bound` is finite and not negative.
    pub fn new(bound: Px<Board>) -> Option<Self> {
        (bound.is_finite() && bound >= Px::<Board>::ZERO).then_some(Self(bound))
    }

    /// `v` held inside `[-bound, bound]`.
    pub fn clamp(self, v: Px<Board>) -> Px<Board> {
        v.clamp(-self.0, self.0)
    }
}

/// The per-import bounds. Defaults are generous headroom over the performance
/// budget (a 5 000-node SVG) while still bounding a pathological file.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum number of elements a single import may create. Guards the R-tree
    /// and memory against an element-count bomb.
    pub max_elements: Count<Elements>,
    /// Maximum group/frame nesting depth walked. Guards against a deeply nested
    /// tree causing unbounded recursion / stack growth.
    pub max_depth: Count<ImportDepth>,
    /// Absolute world-coordinate clamp (applied per axis, symmetric). A coordinate
    /// outside `[-coord_clamp, coord_clamp]` is clamped to the bound.
    pub coord_clamp: CoordBound,
    /// Maximum decoded image size, each edge apart. An image past it on either
    /// edge aborts the import (memory-exhaustion vector).
    pub max_image: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // ~2x the 5 000-node budget: comfortable for real files, still a bound.
            max_elements: Count::<Elements>::new(10_000),
            max_depth: Count::<ImportDepth>::new(64),
            // +/- 1e6 world units: far past any real board, safely inside f32 range
            // for the CRDT geometry fields.
            coord_clamp: CoordBound(Px::<Board>::new(1_000_000.0)),
            // 8192 px: a 8K texture edge; above this the decode cost is abusive.
            max_image: ambre_core::Size::new(Px::new(8_192), Px::new(8_192)),
        }
    }
}

impl Limits {
    /// Clamp one coordinate to `[-coord_clamp, coord_clamp]`. Non-finite input
    /// (NaN/inf from a crafted file) collapses to 0.0 rather than poisoning the geometry.
    pub fn clamp_coord(&self, v: Px<Board>) -> Px<Board> {
        if !v.is_finite() {
            return Px::<Board>::ZERO;
        }
        self.coord_clamp.clamp(v)
    }

    /// Clamp a point to the world bound.
    pub fn clamp_point(&self, p: Point) -> Point {
        Point::new(
            self.clamp_coord(Px::<Board>::new(p.x)).get(),
            self.clamp_coord(Px::<Board>::new(p.y)).get(),
        )
    }

    /// Clamp a rect's origin and far corner to the world bound. Width/height stay
    /// non-negative (a clamp can only shrink, never invert, the box).
    pub fn clamp_rect(&self, r: Rect) -> Rect {
        let low = self.clamp_point(Point::new(r.x0.min(r.x1), r.y0.min(r.y1)));
        let high = self.clamp_point(Point::new(r.x0.max(r.x1), r.y0.max(r.y1)));
        Rect::from_points(low, high)
    }

    /// Fail the import if adding one more element would exceed [`max_elements`].
    /// `current` is the count already accumulated.
    ///
    /// # Errors
    ///
    /// [`ImportError::TooManyElements`], carrying the limit it holds.
    pub fn check_element_count(&self, current: Count<Elements>) -> Result<(), ImportError> {
        if current >= self.max_elements {
            Err(ImportError::TooManyElements {
                limit: self.max_elements,
            })
        } else {
            Ok(())
        }
    }

    /// Fail the import if `depth` exceeds [`max_depth`].
    ///
    /// # Errors
    ///
    /// [`ImportError::TooDeep`], carrying the limit it holds.
    pub fn check_depth(&self, depth: Count<ImportDepth>) -> Result<(), ImportError> {
        if depth > self.max_depth {
            Err(ImportError::TooDeep {
                limit: self.max_depth,
            })
        } else {
            Ok(())
        }
    }

    /// Fail the import if a decoded image exceeds [`max_image`] on either axis.
    ///
    /// # Errors
    ///
    /// [`ImportError::ImageTooLarge`], carrying both dimensions and the limit.
    pub fn check_image_dims(
        &self,
        size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    ) -> Result<(), ImportError> {
        if size.width > self.max_image.width || size.height > self.max_image.height {
            Err(ImportError::ImageTooLarge {
                size,
                limit: self.max_image,
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coord_clamp_bounds_and_sanitizes() {
        let l = bounded(100.0);
        let at = |v: f64| l.clamp_coord(Px::<Board>::new(v)).get();
        assert_eq!(at(50.0), 50.0);
        assert_eq!(at(999.0), 100.0);
        assert_eq!(at(-999.0), -100.0);
        assert_eq!(at(f64::NAN), 0.0);
        assert_eq!(at(f64::INFINITY), 0.0);
    }

    /// A bound that would let every coordinate through, or invert the range,
    /// is refused. The bare `f64::clamp` it replaces panicked on a negative or
    /// NaN bound.
    #[test]
    fn a_coord_bound_is_finite_and_not_negative() {
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(CoordBound::new(Px::<Board>::new(bad)), None, "{bad}");
        }
        let zero = CoordBound::new(Px::<Board>::ZERO).expect("zero is a bound");
        assert_eq!(zero.clamp(Px::<Board>::new(7.0)), Px::<Board>::ZERO);
    }

    fn bounded(bound: f64) -> Limits {
        Limits {
            coord_clamp: CoordBound::new(Px::<Board>::new(bound)).expect("a valid bound"),
            ..Limits::default()
        }
    }

    #[test]
    fn rect_clamp_keeps_non_negative_extent() {
        let l = bounded(100.0);
        let r = l.clamp_rect(Rect::new(-999.0, -999.0, 999.0, 999.0));
        assert_eq!(r, Rect::new(-100.0, -100.0, 100.0, 100.0));
        assert!(r.width() >= 0.0 && r.height() >= 0.0);
    }

    #[test]
    fn element_count_guard() {
        let l = Limits {
            max_elements: Count::<Elements>::new(3),
            ..Limits::default()
        };
        assert!(l.check_element_count(Count::<Elements>::new(2)).is_ok());
        assert!(matches!(
            l.check_element_count(Count::<Elements>::new(3)),
            Err(ImportError::TooManyElements { limit }) if limit == Count::<Elements>::new(3)
        ));
    }

    #[test]
    fn depth_guard() {
        let l = Limits {
            max_depth: Count::<ImportDepth>::new(2),
            ..Limits::default()
        };
        assert!(l.check_depth(Count::<ImportDepth>::new(2)).is_ok());
        assert!(matches!(
            l.check_depth(Count::<ImportDepth>::new(3)),
            Err(ImportError::TooDeep { limit }) if limit == Count::<ImportDepth>::new(2)
        ));
    }

    #[test]
    fn image_dim_guard() {
        let l = Limits {
            max_image: ambre_core::Size::new(Px::new(256), Px::new(256)),
            ..Limits::default()
        };
        assert!(
            l.check_image_dims(ambre_core::Size::new(Px::new(256), Px::new(128)))
                .is_ok()
        );
        assert!(matches!(
            l.check_image_dims(ambre_core::Size::new(Px::new(257), Px::new(10))),
            Err(ImportError::ImageTooLarge { limit, .. }) if limit == ambre_core::Size::new(Px::new(256), Px::new(256))
        ));
    }
}
