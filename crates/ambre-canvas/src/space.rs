// ambre-canvas/src/space.rs
// ============================================================================
// The pixel spaces of this crate alone.
//
// Three spaces: the units of a parley layout, the whole units of a paragraph
// indent, and the size of a glyph raster.
//
// `ambre_core::space` holds the spaces several crates share. These three stay
// here, beside the layout cache and the atlas that measure in them. A length
// of each is an `ambre_core::Px` of the marker.
// ============================================================================

use ambre_core::Space;

/// The units of a parley layout, which are the units of its caller: view
/// pixels for a screen text, document pixels for a board text.
///
/// A drawn glyph run no longer knows which, so the units get a space of
/// their own. A font size, a wrap width and an indent of one layout share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parley {}

impl Space for Parley {
    type Repr = f32;
    const NAME: &'static str = "Parley";
}

/// Whole units of the width a layout breaks to: a paragraph indent
/// (`CANVAS-084`). Whole, so a span keeps its `Eq` and its `Hash` in the
/// cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WholeParley {}

impl Space for WholeParley {
    type Repr = u16;
    const NAME: &'static str = "WholeParley";
}

/// The device pixels a glyph is rasterized at, as the float Swash takes. It
/// is the layout size, times the scale of the transform, times the density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphRaster {}

impl Space for GlyphRaster {
    type Repr = f32;
    const NAME: &'static str = "GlyphRaster";
}
