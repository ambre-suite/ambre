//! Import safety limits (`toile-interop` §2.D / §6 "Import trust").
//!
//! A foreign file is untrusted input: a hostile or merely huge SVG/raster must
//! not exhaust client memory or the R-tree. Every importer threads a [`Limits`]
//! and, on exceeding any bound, aborts the **whole** import atomically -- nothing
//! is applied (§1 invariant). Coordinates are clamped rather than rejected so a
//! merely large (not malicious) drawing still imports, just bounded to the world.

use kurbo::{Point, Rect};

use super::ImportError;

/// The per-import bounds. Defaults are generous headroom over the §8 performance
/// budget (a 5 000-node SVG) while still bounding a pathological file.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum number of elements a single import may create. Guards the R-tree
    /// and memory against an element-count bomb.
    pub max_elements: usize,
    /// Maximum group/frame nesting depth walked. Guards against a deeply nested
    /// tree causing unbounded recursion / stack growth.
    pub max_depth: usize,
    /// Absolute world-coordinate clamp (applied per axis, symmetric). A coordinate
    /// outside `[-coord_clamp, coord_clamp]` is clamped to the bound.
    pub coord_clamp: f64,
    /// Maximum decoded image dimension (width or height, in pixels). A larger
    /// image aborts the import (memory-exhaustion vector, §2.D).
    pub max_image_dim: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // ~2x the 5 000-node budget (§8): comfortable for real files, still a bound.
            max_elements: 10_000,
            max_depth: 64,
            // +/- 1e6 world units: far past any real board, safely inside f32 range
            // for the CRDT geometry fields.
            coord_clamp: 1_000_000.0,
            // 8192 px: a 8K texture edge; above this the decode cost is abusive.
            max_image_dim: 8_192,
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
        v.clamp(-self.coord_clamp, self.coord_clamp)
    }

    /// Clamp a point to the world bound (§2.D).
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
    pub fn check_element_count(&self, current: usize) -> Result<(), ImportError> {
        if current >= self.max_elements {
            Err(ImportError::TooManyElements {
                limit: self.max_elements,
            })
        } else {
            Ok(())
        }
    }

    /// Fail the import if `depth` exceeds [`max_depth`].
    pub fn check_depth(&self, depth: usize) -> Result<(), ImportError> {
        if depth > self.max_depth {
            Err(ImportError::TooDeep {
                limit: self.max_depth,
            })
        } else {
            Ok(())
        }
    }

    /// Fail the import if a decoded image exceeds [`max_image_dim`] on either axis.
    pub fn check_image_dims(&self, width: u32, height: u32) -> Result<(), ImportError> {
        if width > self.max_image_dim || height > self.max_image_dim {
            Err(ImportError::ImageTooLarge {
                width,
                height,
                limit: self.max_image_dim,
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
            coord_clamp: 100.0,
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
            coord_clamp: 100.0,
            ..Limits::default()
        };
        let r = l.clamp_rect(Rect::new(-999.0, -999.0, 999.0, 999.0));
        assert_eq!(r, Rect::new(-100.0, -100.0, 100.0, 100.0));
        assert!(r.width() >= 0.0 && r.height() >= 0.0);
    }

    #[test]
    fn element_count_guard() {
        let l = Limits {
            max_elements: 3,
            ..Limits::default()
        };
        assert!(l.check_element_count(2).is_ok());
        assert!(matches!(
            l.check_element_count(3),
            Err(ImportError::TooManyElements { limit: 3 })
        ));
    }

    #[test]
    fn depth_guard() {
        let l = Limits {
            max_depth: 2,
            ..Limits::default()
        };
        assert!(l.check_depth(2).is_ok());
        assert!(matches!(
            l.check_depth(3),
            Err(ImportError::TooDeep { limit: 2 })
        ));
    }

    #[test]
    fn image_dim_guard() {
        let l = Limits {
            max_image_dim: 256,
            ..Limits::default()
        };
        assert!(l.check_image_dims(256, 128).is_ok());
        assert!(matches!(
            l.check_image_dims(257, 10),
            Err(ImportError::ImageTooLarge { limit: 256, .. })
        ));
    }
}
