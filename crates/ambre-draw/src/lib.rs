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
/// is self-contained.
pub trait DrawSurface {
    /// Fill a rectangle with a solid color.
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// Stroke a rectangle outline.
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64);

    /// Stroke a line segment from (x0, y0) to (x1, y1).
    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64);
}
