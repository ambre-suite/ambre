// ambre-draw/src/types.rs
// Re-exports of geometric and color types shared across rendering backends.
//
// Consumers get a single import path regardless of which underlying crate
// (kurbo, peniko) a type originates from.

pub use kurbo::{Affine, BezPath, Line, Point, Rect, Size, Stroke, Vec2};
pub use peniko::{Brush, BrushRef, Color, Fill};
