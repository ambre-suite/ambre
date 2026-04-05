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
pub use parley::style::StyleProperty;
pub use parley::{Alignment, AlignmentOptions, FontContext, Layout, LayoutContext, PositionedLayoutItem};

/// Brush type used inside Parley layouts across Ambre.
///
/// Parley requires B: Clone + PartialEq + Default + Debug.  peniko::Color
/// (AlphaColor<Srgb>) does not implement Default, so we use raw RGBA bytes
/// as a lightweight token.  The actual render color is passed separately to
/// DrawSurface::draw_glyphs as &peniko::Brush -- the in-layout brush value
/// is not read back by the rendering path.
pub type ParleyBrush = [u8; 4];
