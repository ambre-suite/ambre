// ambre-canvas/src/space.rs
// ============================================================================
// The pixel spaces of this crate alone.
//
// Five spaces:
//
//   * the units of a parley layout;
//   * the whole units of a paragraph indent;
//   * the size of a glyph raster;
//   * the size of a glyph raster in whole hundredths, the key of the atlas;
//   * the whole CSS pixels of the token source.
//
// `ambre_core::space` holds the spaces several crates share. These five stay
// here, as this crate alone measures in them. A length of each is an
// `ambre_core::Px` of the marker.
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
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Parley");
}

/// Whole units of the width a layout breaks to: a paragraph indent
/// (`CANVAS-084`). Whole, so a span keeps its `Eq` and its `Hash` in the
/// cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WholeParley {}

impl Space for WholeParley {
    type Repr = u16;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("WholeParley");
}

/// The device pixels a glyph is rasterized at, as the float Swash takes. It
/// is the layout size, times the scale of the transform, times the density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphRaster {}

impl Space for GlyphRaster {
    type Repr = f32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("GlyphRaster");
}

/// Whole hundredths of a [`GlyphRaster`] pixel: the size the glyph atlas
/// keys a raster by. Whole, so the key keeps its `Eq` and its `Hash`. Two
/// sizes closer than a hundredth share a raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RasterKey {}

impl Space for RasterKey {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RasterKey");
}

/// The whole CSS pixels the design-token source writes: a spacing, a radius,
/// a type size or a shadow (`ambre-spec-design-tokens.md`).
///
/// Whole, so a fractional token cannot be read at all. The stylesheet writes
/// a length with `px`. The Rust module writes it as a `Px<View>`, which
/// `tokens::on_view` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenCss {}

impl Space for TokenCss {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("TokenCss");
}
