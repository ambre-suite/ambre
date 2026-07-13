// ambre-draw/src/types.rs
// Re-exports of geometric, color, and text-layout types shared across rendering backends.
//
// Consumers get a single import path regardless of which underlying crate
// (kurbo, peniko, parley) a type originates from.

pub use kurbo::{Affine, BezPath, Line, Point, Rect, Size, Stroke, Vec2};
pub use peniko::{Brush, BrushRef, Color, Fill};

// Parley types re-exported so that crates implementing or consuming DrawSurface
// do not need a direct parley dependency for the types shared via DrawSurface.
pub use parley::layout::GlyphRun;
pub use parley::style::{FontStyle, FontWeight, StyleProperty};
pub use parley::{
    Alignment, AlignmentOptions, BoundingBox, FontContext, Layout, LayoutContext, PlainEditor,
    PlainEditorDriver, PositionedLayoutItem,
};

/// Brush type used inside Parley layouts across Ambre.
///
/// peniko::Brush (the enum) implements Default (returns Brush::Solid(TRANSPARENT))
/// and satisfies all parley::Brush bounds.  Note: peniko::Color = AlphaColor<Srgb>
/// does NOT implement Default and cannot be used as a layout brush type.
pub type ParleyBrush = Brush;
