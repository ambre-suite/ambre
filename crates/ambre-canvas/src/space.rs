// ambre-canvas/src/space.rs
// ============================================================================
// The pixel spaces of this crate alone.
//
// Eleven spaces:
//
//   * the units of a parley layout;
//   * the whole units of a paragraph indent;
//   * a font size and a wrap width in whole fractions, the key of the text cache;
//   * the size of a glyph raster;
//   * the size of a glyph raster in whole hundredths, the key of the atlas;
//   * the signed bearing of a glyph raster;
//   * the normalized coordinates of a texture;
//   * the units of a path before its transform, and device pixels as a float;
//   * the whole CSS pixels of the token source.
//
// `ambre_core::space` holds the spaces several crates share. These stay
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

/// Whole hundredths of a [`Parley`] unit: the font size the text cache keys a
/// layout by. Whole, so the key keeps its `Eq` and its `Hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParleyHundredths {}

impl Space for ParleyHundredths {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ParleyHundredths");
}

/// Whole tenths of a [`Parley`] unit: the wrap width the text cache keys a
/// layout by. Whole, so the key keeps its `Eq` and its `Hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParleyTenths {}

impl Space for ParleyTenths {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ParleyTenths");
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

/// Whole texels of a glyph raster, signed: the bearing from the glyph origin
/// to an edge of its bitmap, as Swash places it. A bearing points left of the
/// origin or below the baseline as often as not, so the space is signed where
/// [`ambre_core::Texel`] is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TexelShift {}

impl Space for TexelShift {
    type Repr = i32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("TexelShift");
}

/// A coordinate on a texture normalized to its side: 0 at one edge and 1 at
/// the other, as a sampler reads it. A glyph names its box in the atlas in
/// it, so the box survives no change of the atlas side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Uv {}

impl Space for Uv {
    type Repr = f32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Uv");
}

/// The units of a path before the active transform maps it on the screen.
/// The flattening tolerance of a path and the pad of a stroke are measured in
/// them. The `f64` is the one kurbo measures a path in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Model {}

impl Space for Model {
    type Repr = f64;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Model");
}

/// Device pixels as a float: a distance finer than the whole pixel that
/// [`ambre_core::Device`] holds, such as the flattening tolerance of a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FineDevice {}

impl Space for FineDevice {
    type Repr = f64;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("FineDevice");
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
