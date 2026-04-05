// ambre-draw/src/lib.rs
// ============================================================================
// Abstraction trait for shared rendering in Ambre.
//
// `DrawSurface` is the GPU- and PDF-agnostic drawing interface used by
// application-level rendering code (Trame grid, Prose page, ...).
//
// Implementations live in the crates that own the rendering backend:
//   - `VelloSurface` in `ambre-app/src/trame/renderer.rs`  (wraps vello::Scene)
//   - `KrillaSurface` in `ambre-logic-worker/src/trame/`   (wraps krilla, PDF)
//
// Dependency rule: ambre-draw depends only on kurbo + peniko + parley.
// No GPU code, no WASM glue, no vello, no wgpu.
// ============================================================================

pub mod types;
pub use types::*;

/// GPU- and PDF-agnostic drawing interface.
///
/// All coordinates are in logical pixels (f64). The origin is the top-left
/// corner of the rendering surface; Y increases downward.
///
/// The surface is stateless: each `draw_*` call appends a draw command.
/// There is no concept of current position or path accumulation -- each call
/// is self-contained, except for `push_clip`/`pop_clip` which form a stack.
///
/// ## Text rendering
///
/// Text layout (Parley `Layout<Color>`) is performed by the caller: create a
/// `LayoutContext`, shape the text, then iterate over `GlyphRun`s and call
/// `draw_glyphs` for each. This keeps shaping logic out of the trait and allows
/// the caller to cache layouts across frames.
///
/// ## Deferred methods
///
/// `fill_path`, `stroke_path`, `draw_image`, `push_transform`/`pop_transform`
/// are defined in the spec but not yet implemented -- they will be added when
/// `KrillaSurface` (PDF export) is introduced.
pub trait DrawSurface {
    // -------------------------------------------------------------------------
    // Geometry
    // -------------------------------------------------------------------------

    /// Fill a rectangle with a solid color.
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// Stroke a rectangle outline.
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64);

    /// Stroke a line segment from (x0, y0) to (x1, y1).
    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64);

    // -------------------------------------------------------------------------
    // Text
    // -------------------------------------------------------------------------

    /// Render a pre-shaped Parley glyph run.
    ///
    /// `transform` positions the run origin on the surface (typically
    /// `Affine::translate((text_x, text_y))` where text_y is the top of the
    /// text area; the run's own baseline offset is embedded in `run.baseline()`
    /// and already applied by `GlyphRun::positioned_glyphs()`).
    ///
    /// `brush` is the fill color/gradient for the glyphs. For solid text,
    /// use `&Brush::Solid(color)`.
    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine);

    // -------------------------------------------------------------------------
    // Clip stack
    // -------------------------------------------------------------------------

    /// Push a rectangular clip region. All subsequent draw calls are clipped
    /// to `rect` until the matching `pop_clip`.
    ///
    /// Calls must be balanced: every `push_clip` requires exactly one `pop_clip`.
    fn push_clip(&mut self, rect: Rect);

    /// Pop the most recently pushed clip region.
    fn pop_clip(&mut self);
}
