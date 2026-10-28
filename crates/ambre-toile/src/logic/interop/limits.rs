//! Import safety limits (`toile-import-export` "Import trust").
//!
//! A foreign file is untrusted input: a hostile or merely huge SVG/raster must
//! not exhaust client memory or the R-tree. Every importer threads a [`Limits`]
//! and, on exceeding any bound, aborts the **whole** import atomically -- nothing
//! is applied (invariant). Coordinates are clamped rather than rejected so a
//! merely large (not malicious) drawing still imports, just bounded to the world.

use ambre_core::{BoardLength, ElementCount, WholePx};
use kurbo::{Point, Rect};

use super::ImportError;

ambre_core::define_tally! {
    /// How deep an import has walked into nested groups and frames: the root
    /// is depth zero.
    pub struct ImportDepth;
}

/// The per-import bounds. Defaults are generous headroom over the performance
/// budget (a 5 000-node SVG) while still bounding a pathological file.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum number of elements a single import may create. Guards the R-tree
    /// and memory against an element-count bomb.
    pub max_elements: ElementCount,
    /// Maximum group/frame nesting depth walked. Guards against a deeply nested
    /// tree causing unbounded recursion / stack growth.
    pub max_depth: ImportDepth,
    /// Absolute world-coordinate clamp (applied per axis, symmetric). A coordinate
    /// outside `[-coord_clamp, coord_clamp]` is clamped to the bound.
    pub coord_clamp: BoardLength,
    /// Maximum decoded image size, each edge apart. An image past it on either
    /// edge aborts the import (memory-exhaustion vector).
    pub max_image: ambre_core::ImageSize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // ~2x the 5 000-node budget: comfortable for real files, still a bound.
            max_elements: ElementCount::new(10_000),
            max_depth: ImportDepth::new(64),
            // +/- 1e6 world units: far past any real board, safely inside f32 range
            // for the CRDT geometry fields.
            coord_clamp: BoardLength::new(1_000_000.0),
            // 8192 px: a 8K texture edge; above this the decode cost is abusive.
            max_image: ambre_core::ImageSize::new(WholePx::new(8_192), WholePx::new(8_192)),
        }
    }
}

impl Limits {
    /// Clamp one coordinate to `[-coord_clamp, coord_clamp]`. Non-finite input
    /// (NaN/inf from a crafted file) collapses to 0.0 rather than poisoning the geometry.
    pub fn clamp_coord(&self, v: f64) -> f64 {
        if !v.is_finite() {
            return 0.0;
        }
        v.clamp(-self.coord_clamp.get(), self.coord_clamp.get())
    }

    /// Clamp a point to the world bound.
    pub fn clamp_point(&self, p: Point) -> Point {
        Point::new(self.clamp_coord(p.x), self.clamp_coord(p.y))
    }

    /// Clamp a rect's origin and far corner to the world bound. Width/height stay
    /// non-negative (a clamp can only shrink, never invert, the box).
    pub fn clamp_rect(&self, r: Rect) -> Rect {
        let x0 = self.clamp_coord(r.x0.min(r.x1));
        let y0 = self.clamp_coord(r.y0.min(r.y1));
        let x1 = self.clamp_coord(r.x0.max(r.x1));
        let y1 = self.clamp_coord(r.y0.max(r.y1));
        Rect::new(x0, y0, x1, y1)
    }

    /// Fail the import if adding one more element would exceed [`max_elements`].
    /// `current` is the count already accumulated.
    ///
    /// # Errors
    ///
    /// [`ImportError::TooManyElements`], carrying the limit it holds.
    pub fn check_element_count(&self, current: ElementCount) -> Result<(), ImportError> {
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
    pub fn check_depth(&self, depth: ImportDepth) -> Result<(), ImportError> {
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
    pub fn check_image_dims(&self, size: ambre_core::ImageSize) -> Result<(), ImportError> {
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
        let l = Limits {
            coord_clamp: BoardLength::new(100.0),
            ..Limits::default()
        };
        assert_eq!(l.clamp_coord(50.0), 50.0);
        assert_eq!(l.clamp_coord(999.0), 100.0);
        assert_eq!(l.clamp_coord(-999.0), -100.0);
        assert_eq!(l.clamp_coord(f64::NAN), 0.0);
        assert_eq!(l.clamp_coord(f64::INFINITY), 0.0);
    }

    #[test]
    fn rect_clamp_keeps_non_negative_extent() {
        let l = Limits {
            coord_clamp: BoardLength::new(100.0),
            ..Limits::default()
        };
        let r = l.clamp_rect(Rect::new(-999.0, -999.0, 999.0, 999.0));
        assert_eq!(r, Rect::new(-100.0, -100.0, 100.0, 100.0));
        assert!(r.width() >= 0.0 && r.height() >= 0.0);
    }

    #[test]
    fn element_count_guard() {
        let l = Limits {
            max_elements: ElementCount::new(3),
            ..Limits::default()
        };
        assert!(l.check_element_count(ElementCount::new(2)).is_ok());
        assert!(matches!(
            l.check_element_count(ElementCount::new(3)),
            Err(ImportError::TooManyElements { limit }) if limit == ElementCount::new(3)
        ));
    }

    #[test]
    fn depth_guard() {
        let l = Limits {
            max_depth: ImportDepth::new(2),
            ..Limits::default()
        };
        assert!(l.check_depth(ImportDepth::new(2)).is_ok());
        assert!(matches!(
            l.check_depth(ImportDepth::new(3)),
            Err(ImportError::TooDeep { limit }) if limit == ImportDepth::new(2)
        ));
    }

    #[test]
    fn image_dim_guard() {
        let l = Limits {
            max_image: ambre_core::ImageSize::new(WholePx::new(256), WholePx::new(256)),
            ..Limits::default()
        };
        assert!(
            l.check_image_dims(ambre_core::ImageSize::new(
                WholePx::new(256),
                WholePx::new(128)
            ))
            .is_ok()
        );
        assert!(matches!(
            l.check_image_dims(ambre_core::ImageSize::new(WholePx::new(257), WholePx::new(10))),
            Err(ImportError::ImageTooLarge { limit, .. }) if limit == ambre_core::ImageSize::new(WholePx::new(256), WholePx::new(256))
        ));
    }
}
