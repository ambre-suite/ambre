// ambre-toile/src/ui/viewport.rs
// ============================================================================
// ViewportTransform: the render thread's world <-> screen camera (toile-viewport).
//
// Deliberately a distinct type from `ambre_bus::app::toile::Viewport` (the Logic
// culling input). They carry identical fields, but this one is the GPU transform.
// It is updated locally and immediately on every pan / zoom for a jank-free
// gesture (toile-viewport). The UI forwards each change to Logic as `SetViewport`,
// but never waits on it to paint. Pure geometry -- host-unit-tested, no
// `cfg(wasm32)`.
// ============================================================================

use ambre_bus::app::toile::Viewport;
/// Minimum / maximum zoom: the bounds the culling type clamps to
/// (`TOILE-UI-281`), so the camera and the Logic thread share one range.
pub use ambre_bus::app::toile::{MAX_ZOOM, MIN_ZOOM};
use ambre_core::ZoomFactor;
use ambre_draw::kinetic::ScrollStep;
use ambre_draw::{Affine, Point, Rect, Size, Vec2};

/// One press of the zoom-in / zoom-out button multiplies (or divides) the zoom
/// by this factor. A discrete notch, coarser than a wheel tick.
pub const ZOOM_STEP: f64 = 1.2;

/// Fractional breathing room left around the fitted content on a Fit /
/// Zoom-to-selection. The content occupies `1 / (1 + FIT_PAD)` of each
/// axis, so it never touches the viewport edge.
pub const FIT_PAD: f64 = 0.08;

/// The world <-> screen camera. `center` is the world point under the screen
/// center; `zoom` is world-units-per... no: screen = (world - center) * zoom +
/// size/2, so `zoom` is screen-pixels per world-unit. `size` is logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportTransform {
    pub center: Point,
    pub zoom: ZoomFactor,
    pub size: Size,
}

impl Default for ViewportTransform {
    fn default() -> Self {
        Self {
            center: Point::ZERO,
            zoom: ZoomFactor::ONE,
            size: Size::new(1280.0, 800.0),
        }
    }
}

impl ViewportTransform {
    /// Build from a logical canvas size, centered on the world origin at 1x.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            center: Point::ZERO,
            zoom: ZoomFactor::ONE,
            size: Size::new(width.max(1.0), height.max(1.0)),
        }
    }

    /// The affine mapping a world point to a screen (logical-pixel) point:
    /// `screen = translate(size/2) * scale(zoom) * translate(-center)` applied to
    /// the world point (rightmost factor runs first, kurbo convention).
    pub fn world_to_screen(&self) -> Affine {
        let half = Vec2::new(self.size.width / 2.0, self.size.height / 2.0);
        Affine::translate(half)
            * Affine::scale(self.zoom.get())
            * Affine::translate(-self.center.to_vec2())
    }

    /// The inverse mapping (screen -> world). Always invertible: `zoom > 0`.
    pub fn screen_to_world(&self) -> Affine {
        self.world_to_screen().inverse()
    }

    /// Clamp a zoom request to the supported range.
    fn clamp_zoom(z: f64) -> ZoomFactor {
        ZoomFactor::within(z, MIN_ZOOM.get(), MAX_ZOOM.get())
    }

    /// Zoom by `factor` about a screen anchor (typically the cursor), keeping the
    /// world point currently under the anchor fixed on screen. A factor > 1
    /// zooms in. The zoom is clamped; at a clamp boundary the anchor still holds
    /// (the recomputed center uses the clamped zoom).
    pub fn zoom_about(&mut self, screen_anchor: Point, factor: f64) {
        let world_anchor = self.screen_to_world() * screen_anchor;
        let new_zoom = Self::clamp_zoom(self.zoom.get() * factor);
        // Solve `(world_anchor - center) * new_zoom + size/2 == screen_anchor`
        // for center so the anchored world point stays under the cursor.
        let half = Vec2::new(self.size.width / 2.0, self.size.height / 2.0);
        let offset = (screen_anchor.to_vec2() - half) / new_zoom.get();
        self.zoom = new_zoom;
        self.center = world_anchor - offset;
    }

    /// Zoom by `factor` about the viewport center: the zoom-in / zoom-out button
    /// affordance. Unlike `zoom_about` this keeps the world point under the
    /// screen center fixed, so the board scales in place. Clamped.
    pub fn zoom_by_center(&mut self, factor: f64) {
        let center_screen = Point::new(self.size.width / 2.0, self.size.height / 2.0);
        self.zoom_about(center_screen, factor);
    }

    /// Reset to 100% (1 screen-pixel per world-unit) keeping the current center
    /// under the screen center (the `%` chip click,).
    pub fn reset_zoom(&mut self) {
        self.zoom = ZoomFactor::ONE;
    }

    /// Fit `world` into the viewport with `pad_frac` breathing room on each axis,
    /// centering the camera on the rect (Fit / Zoom-to-selection). A zero-area or
    /// non-finite rect is a no-op (nothing to frame). The fit zoom is clamped to
    /// the supported range, so a tiny rect does not blow past `MAX_ZOOM`.
    /// `pad_frac` grows the target extent so the content occupies `1 / (1 +
    /// pad_frac)` of the axis.
    pub fn fit_bounds(&mut self, world: Rect, pad_frac: f64) {
        if let Some((center, zoom)) = self.fit_target(world, pad_frac) {
            self.center = center;
            self.zoom = zoom;
        }
    }

    /// The `(center, zoom)` a [`fit_bounds`](Self::fit_bounds) would settle on,
    /// without mutating the camera: the pure fit solver. `None` for a zero-area
    /// or non-finite rect (nothing to frame). Shared by the instant Fit and by
    /// the presentation camera glide, which lerps toward this target.
    pub fn fit_target(&self, world: Rect, pad_frac: f64) -> Option<(Point, ZoomFactor)> {
        let (bw, bh) = (world.width(), world.height());
        if !(bw.is_finite() && bh.is_finite()) || bw <= 0.0 || bh <= 0.0 {
            return None;
        }
        let pad = 1.0 + pad_frac.max(0.0);
        let fit_x = self.size.width / (bw * pad);
        let fit_y = self.size.height / (bh * pad);
        Some((world.center(), Self::clamp_zoom(fit_x.min(fit_y))))
    }

    /// Interpolate the camera from `from` toward a target center + zoom at phase
    /// `t` (already eased), keeping the current `size` (the presentation glide of
    /// `toile-presentation`). `t = 0` reproduces `from`, `t = 1` lands exactly on the
    /// target.
    /// Pure -- host-tested; the caller assigns the result to `self`.
    pub fn glide(
        from: ViewportTransform,
        to_center: Point,
        to_zoom: ZoomFactor,
        t: f64,
    ) -> ViewportTransform {
        ViewportTransform {
            center: Point::new(
                from.center.x + (to_center.x - from.center.x) * t,
                from.center.y + (to_center.y - from.center.y) * t,
            ),
            zoom: ZoomFactor::positive(from.zoom.get() + (to_zoom.get() - from.zoom.get()) * t),
            size: from.size,
        }
    }

    /// Move the view by a step of the shared model, in screen pixels. The world
    /// center follows the step, scaled down by the current zoom, so a drag
    /// moves the content under the cursor by the drag. Only the model builds a
    /// step, so a distance the board computed itself does not move the camera
    /// (`KINETIC-001`).
    pub fn pan_by_screen(&mut self, step: ScrollStep) {
        self.center += Vec2::new(f64::from(step.x()), f64::from(step.y())) / self.zoom.get();
    }

    /// Set the logical canvas size (on resize). Keeps center and zoom.
    pub fn set_size(&mut self, width: f64, height: f64) {
        self.size = Size::new(width.max(1.0), height.max(1.0));
    }

    /// The world rectangle this camera shows.
    pub fn visible_world(&self) -> Rect {
        let half_w = self.size.width / (2.0 * self.zoom.get());
        let half_h = self.size.height / (2.0 * self.zoom.get());
        Rect::new(
            self.center.x - half_w,
            self.center.y - half_h,
            self.center.x + half_w,
            self.center.y + half_h,
        )
    }

    /// A culling viewport covering what this camera shows and what the camera
    /// `(to_center, to_zoom)` will (`TOILE-UI-085`).
    ///
    /// A slide transition glides between the two, and the culled set is asked
    /// for once per frame. Asking at the moving camera drops an element the
    /// glide leaves behind, which stops its fade-out halfway. The union keeps
    /// every element of both slides in the set for the whole transition, and
    /// adds nothing that is off both screens.
    pub fn covering_viewport(&self, to_center: Point, to_zoom: ZoomFactor) -> Viewport {
        let to = ViewportTransform {
            center: to_center,
            zoom: to_zoom,
            size: self.size,
        };
        let union = self.visible_world().union(to.visible_world());
        let zoom_x = self.size.width / union.width().max(f64::EPSILON);
        let zoom_y = self.size.height / union.height().max(f64::EPSILON);
        Viewport {
            center: union.center(),
            zoom: Self::clamp_zoom(zoom_x.min(zoom_y)),
            size: self.size,
        }
    }

    /// Project to the Logic-side `Viewport` for a `SetViewport` forward (the
    /// culling input). Identical fields, a distinct type by design.
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
            zoom: ambre_core::ZoomFactor::positive(2.5),
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
            zoom: ambre_core::ZoomFactor::positive(3.0),
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
        assert!((vt.zoom.get() - 1.8).abs() < 1e-9);
    }

    #[test]
    fn zoom_clamps_to_range() {
        let mut vt = ViewportTransform::new(1000.0, 1000.0);
        for _ in 0..100 {
            vt.zoom_about(Point::new(500.0, 500.0), 2.0);
        }
        assert!((vt.zoom.get() - MAX_ZOOM.get()).abs() < 1e-9);
        for _ in 0..200 {
            vt.zoom_about(Point::new(500.0, 500.0), 0.5);
        }
        assert!((vt.zoom.get() - MIN_ZOOM.get()).abs() < 1e-9);
    }

    #[test]
    fn zoom_by_center_keeps_center_world_point_fixed() {
        let mut vt = ViewportTransform::new(1000.0, 800.0);
        vt.center = Point::new(42.0, -13.0);
        let center_before = vt.center;
        vt.zoom_by_center(ZOOM_STEP);
        // The world point under the screen center is the camera center itself; it
        // must not move when zooming about the center.
        assert!(approx(vt.center, center_before));
        assert!((vt.zoom.get() - ZOOM_STEP).abs() < 1e-9);
    }

    #[test]
    fn reset_zoom_returns_to_100_percent() {
        let mut vt = ViewportTransform::new(1000.0, 800.0);
        vt.zoom = ambre_core::ZoomFactor::positive(4.7);
        vt.center = Point::new(10.0, 20.0);
        vt.reset_zoom();
        assert!((vt.zoom.get() - 1.0).abs() < 1e-9);
        // Center is preserved (the chip resets scale, not position).
        assert!(approx(vt.center, Point::new(10.0, 20.0)));
    }

    #[test]
    fn fit_bounds_centers_and_scales_with_padding() {
        let mut vt = ViewportTransform::new(1000.0, 800.0);
        // A 200x100 world rect centered at (150, 60).
        let world = Rect::new(50.0, 10.0, 250.0, 110.0);
        vt.fit_bounds(world, FIT_PAD);
        assert!(approx(vt.center, world.center()));
        // Fit is the tighter axis: height (800 / (100 * 1.08)) vs width
        // (1000 / (200 * 1.08)); the min governs.
        let pad = 1.0 + FIT_PAD;
        let expected = (1000.0 / (200.0 * pad)).min(800.0 / (100.0 * pad));
        assert!((vt.zoom.get() - expected).abs() < 1e-9);
        // The padded content fits inside the viewport on both axes.
        assert!(world.width() * vt.zoom.get() <= vt.size.width + 1e-6);
        assert!(world.height() * vt.zoom.get() <= vt.size.height + 1e-6);
    }

    #[test]
    fn fit_bounds_clamps_tiny_rect_to_max_zoom() {
        let mut vt = ViewportTransform::new(1000.0, 800.0);
        // A sub-pixel rect would demand an enormous zoom; it must clamp.
        vt.fit_bounds(Rect::new(0.0, 0.0, 0.001, 0.001), FIT_PAD);
        assert!((vt.zoom.get() - MAX_ZOOM.get()).abs() < 1e-9);
    }

    #[test]
    fn fit_bounds_ignores_empty_rect() {
        let mut vt = ViewportTransform::new(1000.0, 800.0);
        let before = vt;
        vt.fit_bounds(Rect::new(5.0, 5.0, 5.0, 5.0), FIT_PAD); // zero area
        assert_eq!(vt, before);
    }

    #[test]
    fn fit_target_matches_fit_bounds() {
        let vt = ViewportTransform::new(1000.0, 800.0);
        let world = Rect::new(50.0, 10.0, 250.0, 110.0);
        let (center, zoom) = vt.fit_target(world, FIT_PAD).unwrap();
        // The pure solver returns exactly what the in-place fit settles on.
        let mut moved = vt;
        moved.fit_bounds(world, FIT_PAD);
        assert!(approx(center, moved.center));
        assert!((zoom.get() - moved.zoom.get()).abs() < 1e-9);
        // A zero-area rect yields no target.
        assert!(
            vt.fit_target(Rect::new(5.0, 5.0, 5.0, 5.0), FIT_PAD)
                .is_none()
        );
    }

    #[test]
    fn glide_interpolates_center_and_zoom() {
        let from = ViewportTransform {
            center: Point::new(0.0, 0.0),
            zoom: ambre_core::ZoomFactor::positive(1.0),
            size: Size::new(1000.0, 800.0),
        };
        let to_center = Point::new(100.0, 200.0);
        let to_zoom = ambre_core::ZoomFactor::positive(3.0);
        // t = 0 reproduces the source exactly.
        let g0 = ViewportTransform::glide(from, to_center, to_zoom, 0.0);
        assert!(approx(g0.center, from.center));
        assert!((g0.zoom.get() - from.zoom.get()).abs() < 1e-9);
        // t = 1 lands exactly on the target.
        let g1 = ViewportTransform::glide(from, to_center, to_zoom, 1.0);
        assert!(approx(g1.center, to_center));
        assert!((g1.zoom.get() - to_zoom.get()).abs() < 1e-9);
        // Midpoint is the linear halfway (easing is applied by the caller).
        let gm = ViewportTransform::glide(from, to_center, to_zoom, 0.5);
        assert!(approx(gm.center, Point::new(50.0, 100.0)));
        assert!((gm.zoom.get() - 2.0).abs() < 1e-9);
        // Size is carried through unchanged.
        assert_eq!(gm.size, from.size);
    }

    #[test]
    fn pan_moves_content_with_the_drag() {
        let mut vt = ViewportTransform::new(1000.0, 1000.0);
        vt.zoom = ambre_core::ZoomFactor::positive(2.0);
        let world_pt = Point::new(50.0, 50.0);
        let screen_before = vt.world_to_screen() * world_pt;
        vt.pan_by_screen(ScrollStep::dragged((0.0, 0.0), (30.0, -20.0)));
        let screen_after = vt.world_to_screen() * world_pt;
        // A pan of (30, -20) screen px moves every painted world point by the
        // same screen delta.
        assert!((screen_after.x - screen_before.x - 30.0).abs() < 1e-6);
        assert!((screen_after.y - screen_before.y + 20.0).abs() < 1e-6);
    }

    /// `TOILE-UI-285`: a zoom past a stop leaves the zoom at the stop, and the
    /// world point under the anchor stays under it, at both stops.
    #[test]
    fn a_zoom_at_a_stop_holds_its_anchor() {
        for (zoom, factor) in [(MAX_ZOOM, 4.0), (MIN_ZOOM, 0.25)] {
            let mut vt = ViewportTransform::new(1000.0, 800.0);
            vt.zoom = zoom;
            vt.center = Point::new(120.0, -40.0);
            let anchor = Point::new(810.0, 95.0);
            let under = vt.screen_to_world() * anchor;
            vt.zoom_about(anchor, factor);
            assert_eq!(vt.zoom, zoom, "the stop refuses the change");
            assert!(
                approx(vt.world_to_screen() * under, anchor),
                "zoom {zoom:?}"
            );
        }
    }

    /// `TOILE-UI-291`: a fit over an extent that is not finite leaves the
    /// camera as it was, as a zero-area fit does.
    #[test]
    fn a_fit_over_an_extent_that_is_not_finite_is_refused() {
        let mut vt = ViewportTransform::new(1000.0, 800.0);
        vt.center = Point::new(3.0, 4.0);
        let before = vt;
        for world in [
            Rect::new(0.0, 0.0, f64::INFINITY, 10.0),
            Rect::new(f64::NEG_INFINITY, 0.0, 10.0, 10.0),
            Rect::new(0.0, 0.0, 10.0, f64::NAN),
            Rect::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN),
        ] {
            assert_eq!(vt.fit_target(world, FIT_PAD), None, "{world:?}");
            vt.fit_bounds(world, FIT_PAD);
            assert_eq!(vt, before, "{world:?}");
        }
    }
}
