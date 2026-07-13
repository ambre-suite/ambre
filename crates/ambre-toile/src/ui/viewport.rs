// ambre-toile/src/ui/viewport.rs
// ============================================================================
// ViewportTransform: the render thread's world <-> screen camera (toile-ui §3).
//
// Deliberately a distinct type from `ambre_bus::app::toile::Viewport` (the Logic
// culling input): they carry identical fields but this one is the GPU transform,
// updated locally and immediately on every pan / zoom for a jank-free gesture
// (toile-ui §2). The UI forwards each change to Logic as `SetViewport`, but never
// waits on it to paint. Pure geometry -- host-unit-tested, no `cfg(wasm32)`.
// ============================================================================

use ambre_bus::app::toile::Viewport;
use ambre_draw::{Affine, Point, Size, Vec2};

/// Minimum / maximum zoom, matching the Logic-side clamp (`Viewport`, §3).
pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 10.0;

/// The world <-> screen camera. `center` is the world point under the screen
/// center; `zoom` is world-units-per... no: screen = (world - center) * zoom +
/// size/2, so `zoom` is screen-pixels per world-unit. `size` is physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportTransform {
    pub center: Point,
    pub zoom: f64,
    pub size: Size,
}

impl Default for ViewportTransform {
    fn default() -> Self {
        Self {
            center: Point::ZERO,
            zoom: 1.0,
            size: Size::new(1280.0, 800.0),
        }
    }
}

impl ViewportTransform {
    /// Build from a physical canvas size, centered on the world origin at 1x.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            center: Point::ZERO,
            zoom: 1.0,
            size: Size::new(width.max(1.0), height.max(1.0)),
        }
    }

    /// The affine mapping a world point to a screen (physical-pixel) point:
    /// `screen = translate(size/2) * scale(zoom) * translate(-center)` applied to
    /// the world point (rightmost factor runs first, kurbo convention).
    pub fn world_to_screen(&self) -> Affine {
        let half = Vec2::new(self.size.width / 2.0, self.size.height / 2.0);
        Affine::translate(half)
            * Affine::scale(self.zoom)
            * Affine::translate(-self.center.to_vec2())
    }

    /// The inverse mapping (screen -> world). Always invertible: `zoom > 0`.
    pub fn screen_to_world(&self) -> Affine {
        self.world_to_screen().inverse()
    }

    /// Clamp a zoom request to the supported range.
    fn clamp_zoom(z: f64) -> f64 {
        z.clamp(MIN_ZOOM, MAX_ZOOM)
    }

    /// Zoom by `factor` about a screen anchor (typically the cursor), keeping the
    /// world point currently under the anchor fixed on screen (§3). A factor > 1
    /// zooms in. The zoom is clamped; at a clamp boundary the anchor still holds
    /// (the recomputed center uses the clamped zoom).
    pub fn zoom_about(&mut self, screen_anchor: Point, factor: f64) {
        let world_anchor = self.screen_to_world() * screen_anchor;
        let new_zoom = Self::clamp_zoom(self.zoom * factor);
        // Solve `(world_anchor - center) * new_zoom + size/2 == screen_anchor`
        // for center so the anchored world point stays under the cursor.
        let half = Vec2::new(self.size.width / 2.0, self.size.height / 2.0);
        let offset = (screen_anchor.to_vec2() - half) / new_zoom;
        self.zoom = new_zoom;
        self.center = world_anchor - offset;
    }

    /// Pan the camera by a screen-space delta (a drag of `delta` physical pixels
    /// moves the content under the cursor by the same amount): the world center
    /// shifts against the drag, scaled down by the current zoom.
    pub fn pan_by_screen(&mut self, delta: Vec2) {
        self.center -= delta / self.zoom;
    }

    /// Set the physical canvas size (on resize). Keeps center and zoom.
    pub fn set_size(&mut self, width: f64, height: f64) {
        self.size = Size::new(width.max(1.0), height.max(1.0));
    }

    /// Project to the Logic-side `Viewport` for a `SetViewport` forward (the
    /// culling input). Identical fields, a distinct type by design (§3).
    pub fn to_bus_viewport(&self) -> Viewport {
        Viewport {
            center: self.center,
            zoom: self.zoom,
            size: self.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    #[test]
    fn round_trip_is_identity() {
        let vt = ViewportTransform {
            center: Point::new(120.0, -45.0),
            zoom: 2.5,
            size: Size::new(1000.0, 600.0),
        };
        let w2s = vt.world_to_screen();
        let s2w = vt.screen_to_world();
        for p in [
            Point::new(0.0, 0.0),
            Point::new(300.0, 200.0),
            Point::new(-50.0, 999.0),
        ] {
            let back = s2w * (w2s * p);
            assert!(approx(back, p), "round-trip failed for {p:?} -> {back:?}");
        }
    }

    #[test]
    fn center_maps_to_screen_center() {
        let vt = ViewportTransform {
            center: Point::new(10.0, 20.0),
            zoom: 3.0,
            size: Size::new(800.0, 400.0),
        };
        let s = vt.world_to_screen() * vt.center;
        assert!(approx(s, Point::new(400.0, 200.0)));
    }

    #[test]
    fn zoom_about_keeps_anchor_world_point_fixed() {
        let mut vt = ViewportTransform::new(1200.0, 800.0);
        let anchor = Point::new(300.0, 250.0);
        let world_before = vt.screen_to_world() * anchor;
        vt.zoom_about(anchor, 1.8);
        let world_after = vt.screen_to_world() * anchor;
        assert!(
            approx(world_before, world_after),
            "anchor world point moved: {world_before:?} -> {world_after:?}"
        );
        assert!((vt.zoom - 1.8).abs() < 1e-9);
    }

    #[test]
    fn zoom_clamps_to_range() {
        let mut vt = ViewportTransform::new(1000.0, 1000.0);
        for _ in 0..100 {
            vt.zoom_about(Point::new(500.0, 500.0), 2.0);
        }
        assert!((vt.zoom - MAX_ZOOM).abs() < 1e-9);
        for _ in 0..200 {
            vt.zoom_about(Point::new(500.0, 500.0), 0.5);
        }
        assert!((vt.zoom - MIN_ZOOM).abs() < 1e-9);
    }

    #[test]
    fn pan_moves_content_with_the_drag() {
        let mut vt = ViewportTransform::new(1000.0, 1000.0);
        vt.zoom = 2.0;
        let world_pt = Point::new(50.0, 50.0);
        let screen_before = vt.world_to_screen() * world_pt;
        vt.pan_by_screen(Vec2::new(30.0, -20.0));
        let screen_after = vt.world_to_screen() * world_pt;
        // A pan of (30, -20) screen px moves every painted world point by the
        // same screen delta.
        assert!((screen_after.x - screen_before.x - 30.0).abs() < 1e-6);
        assert!((screen_after.y - screen_before.y + 20.0).abs() < 1e-6);
    }
}
