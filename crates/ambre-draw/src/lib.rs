// ambre-draw/src/lib.rs
// ============================================================================
// Abstraction trait for shared rendering in Ambre.
//
// `DrawSurface` is the GPU- and PDF-agnostic drawing interface used by
// application-level rendering code (Trame grid, Prose page, Toile canvas, ...).
//
// Implementations:
//   - `WgpuSurface`   in `ambre-canvas` (lyon tessellation + wgpu, on-screen) — live.
//   - `KrillaSurface` in `ambre-pdf` (krilla, PDF export, Logic thread, lazy-loaded) — live.
//
// Dependency rule: ambre-draw depends only on kurbo + peniko + parley.
// No GPU code, no WASM glue, no wgpu, no lyon, no krilla. This keeps the core
// drawing logic portable and testable.
// ============================================================================

pub mod types;
pub use types::*;

#[cfg(feature = "testing")]
pub mod testing;

/// Monotonic millisecond clock, abstracted so rendering code can measure its
/// sub-phase timings without depending on `web_sys::Performance` directly.
///
/// On the UI Thread the implementation wraps `performance.now()`; headless
/// tests pass a deterministic stub (see [`testing::FixedClock`]). This keeps the
/// renderers callable off the browser main thread, which is what makes the
/// mock-surface tests possible.
pub trait Clock {
    /// Wall-clock milliseconds, monotonic within a frame. Sub-millisecond
    /// precision is expected on screen but never relied upon for correctness.
    fn now_ms(&self) -> f64;
}

/// GPU- and PDF-agnostic drawing interface.
///
/// All coordinates are in logical pixels (f64). The origin is the top-left
/// corner of the rendering surface; Y increases downward.
///
/// Each `draw_*` call appends a draw command -- the surface is append-only
/// within a frame. The only stateful mechanism is the clip stack
/// (`push_clip`/`pop_clip`) and the transform stack
/// (`push_transform`/`pop_transform`), which must always be balanced.
///
/// ## Text rendering
///
/// Text layout (Parley `Layout<ParleyBrush>`) is the caller's responsibility.
/// Shape once, cache the `Layout`, then call `draw_glyphs` for each `GlyphRun`
/// every frame. This keeps shaping out of the trait and enables per-frame
/// zero-allocation rendering on cache hits.
///
/// ## Path rendering
///
/// `fill_path` and `stroke_path` accept `kurbo::BezPath`. On screen this is
/// tessellated by lyon; in PDF it maps to krilla path primitives. Callers never
/// depend on lyon directly.
pub trait DrawSurface {
    // -------------------------------------------------------------------------
    // Geometry -- primitives
    // -------------------------------------------------------------------------

    /// Fill a rectangle with a solid color.
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// Stroke a rectangle outline.
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64);

    /// Stroke a line segment.
    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64);

    // -------------------------------------------------------------------------
    // Text
    // -------------------------------------------------------------------------

    /// Render a pre-shaped Parley glyph run at the given transform.
    ///
    /// `transform` positions the run origin (typically
    /// `Affine::translate((x, baseline_y))`). The run's own advance offsets
    /// are embedded in `GlyphRun::positioned_glyphs()`.
    ///
    /// On screen: glyphs are rasterized via Swash into a GPU atlas and drawn
    /// as textured quads. In PDF: mapped to krilla glyph placement.
    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine);

    // -------------------------------------------------------------------------
    // Clip stack
    // -------------------------------------------------------------------------

    /// Push a rectangular clip. All subsequent draw calls are scissored to
    /// `rect` until the matching `pop_clip`. Calls must be balanced.
    fn push_clip(&mut self, rect: Rect);

    /// Pop the most recently pushed clip region.
    fn pop_clip(&mut self);

    // -------------------------------------------------------------------------
    // Geometry -- arbitrary paths (default no-ops until lyon is wired)
    // -------------------------------------------------------------------------

    /// Fill an arbitrary path. Default no-op -- override when lyon is available.
    #[allow(unused_variables)]
    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {}

    /// Stroke an arbitrary path. Default no-op -- override when lyon is available.
    #[allow(unused_variables)]
    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {}

    // -------------------------------------------------------------------------
    // Transform stack (default no-ops until Toile needs it)
    // -------------------------------------------------------------------------

    /// Push an affine transform. Subsequent draw calls are in the transformed
    /// coordinate space. Calls must be balanced with `pop_transform`.
    ///
    /// Used by Toile (infinite canvas pan/zoom) and Prose (page margins).
    /// Default no-op -- override when transform support is implemented.
    #[allow(unused_variables)]
    fn push_transform(&mut self, transform: Affine) {}

    /// Pop the most recently pushed transform. Default no-op.
    fn pop_transform(&mut self) {}
}
