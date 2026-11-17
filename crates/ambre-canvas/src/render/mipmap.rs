// render/mipmap.rs
// ============================================================================
// The mip chain of a decoded image, built on the CPU.
//
// * A picture on a board is almost always minified. A photograph is thousands
//   of texels wide, and its element box is a few hundred pixels. Sampling only
//   level 0 makes each screen pixel take one texel out of many. The picture
//   then crawls as the board pans, the most visible defect a raster path can
//   have. A mip chain is what a trilinear sampler needs to average them
//   instead.
// * The CPU builds the chain rather than the GPU, on purpose. Render passes
//   would cost one pass per level per upload, and a compute shader would cost
//   a dispatch. `ambre-spec-frame.md section 4` refuses that stage, and a
//   WebGL2 back end does not have it.
// * The upload path writes an image once, then draws it for as long as it
//   stays on screen. So the CPU pays once and no frame pays.
// ============================================================================

use ambre_core::{Count, Image, NarrowCount, Octets, Payload, Px, RgbaPixels, Size};

/// One level of a mip chain: RGBA8 samples and the dimensions they describe.
pub struct MipLevel {
    pub pixels: Payload<RgbaPixels>,
    pub size: Size<Px<Image>>,
}

/// The levels of a mip chain, level 0 included, as wgpu counts them.
pub enum MipLevels {}

impl NarrowCount for MipLevels {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("MipLevels");
}

/// The bytes level 0 of an image of `size` holds: four per pixel, RGBA8.
pub fn level0_size(size: Size<Px<Image>>) -> Count<Octets> {
    // In `u64` and saturating: a `usize` product wraps on wasm32 for a large
    // declared size, and a wrapped size would admit a short buffer.
    Count::<Octets>::new(
        u64::from(size.width.get())
            .saturating_mul(u64::from(size.height.get()))
            .saturating_mul(4),
    )
}

/// Build the levels *below* level 0 by repeated 2x2 box reduction, stopping at
/// 1x1. Level 0 is not copied: the caller already holds those pixels and this
/// avoids duplicating a multi-megabyte buffer to upload it.
///
/// Each dimension halves independently and floors at 1, the rule wgpu itself
/// uses to count levels. A 1024x16 image therefore yields a chain that stays
/// 1 texel tall while its width keeps halving.
///
/// `pixels` shorter than `width * height * 4` yields no levels at all: a
/// malformed source is a caller bug the upload path already declines to guess at.
pub fn build_chain(pixels: &Payload<RgbaPixels>, size: Size<Px<Image>>) -> Vec<MipLevel> {
    let mut out = Vec::new();
    let (width, height) = (size.width.get(), size.height.get());
    if width == 0 || height == 0 {
        return out;
    }
    if pixels.len() < level0_size(size) {
        return out;
    }
    let mut src = pixels.clone();
    let mut from = size;
    let one = Px::<Image>::new(1);
    while from.width > one || from.height > one {
        let half = |edge: Px<Image>| Px::<Image>::new(edge.get() / 2).max(one);
        let to = Size::new(half(from.width), half(from.height));
        src = reduce(&src, from, to);
        out.push(MipLevel {
            pixels: src.clone(),
            size: to,
        });
        from = to;
    }
    out
}

/// The number of mip levels wgpu expects for an image of this size, level 0
/// included -- `floor(log2(max(w, h))) + 1`, and 1 for a degenerate size.
pub fn level_count(size: Size<Px<Image>>) -> Count<MipLevels> {
    let longest = size.width.max(size.height).get();
    if longest <= 1 {
        return Count::new(1);
    }
    Count::new(32 - longest.leading_zeros())
}

/// The sampler a picture is drawn through.
///
/// Trilinear: linear within a mip level and linear between two. A picture
/// shrinking under a zoom then crosses levels without the step a `Nearest`
/// mipmap filter leaves (`CANVAS-065`). `ClampToEdge` keeps the border texel from
/// wrapping onto the opposite edge of the quad.
pub fn sampler() -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("image_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    }
}

/// Average each 2x2 block of `src` into one texel of the destination.
///
/// * The average stays in the sample space the pixels arrive in, never in
///   linear light. `ImageSource` carries non-premultiplied sRGB RGBA8 and the
///   pipeline blends it as such. Converting here would make a reduced level
///   disagree with the level above it.
/// * An odd dimension leaves a final row or column with no partner. Each such
///   texel takes the block that starts at its own doubled coordinate, clamped.
///   The clamp carries the edge rather than dropping it.
fn reduce(
    src: &Payload<RgbaPixels>,
    from: Size<Px<Image>>,
    to: Size<Px<Image>>,
) -> Payload<RgbaPixels> {
    let (w, h) = (from.width.get(), from.height.get());
    let (nw, nh) = (to.width.get(), to.height.get());
    let src = src.as_bytes();
    let mut dst = vec![0u8; (nw as usize) * (nh as usize) * 4];
    // A texel past the end of `src` reads as zero. The caller sizes `src` as
    // `w * h` texels, so none is; the read stays checked all the same.
    let at = |px: u32, py: u32, c: usize| -> u32 {
        src.get(((py * w + px) as usize) * 4 + c)
            .map_or(0, |&v| u32::from(v))
    };
    for (i, texel) in dst.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i as u32 % nw, i as u32 / nw);
        // The 2x2 block this destination texel averages. The clamp makes an
        // odd source dimension read its last row or column twice instead of
        // sampling past the end.
        let x0 = (x * 2).min(w - 1);
        let y0 = (y * 2).min(h - 1);
        let x1 = (x * 2 + 1).min(w - 1);
        let y1 = (y * 2 + 1).min(h - 1);
        for (c, out) in texel.iter_mut().enumerate() {
            let sum = at(x0, y0, c) + at(x1, y0, c) + at(x0, y1, c) + at(x1, y1, c);
            *out = ((sum + 2) / 4) as u8;
        }
    }
    Payload::new(dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{Image, Px, Size};

    fn size(width: u32, height: u32) -> Size<Px<Image>> {
        Size::new(Px::new(width), Px::new(height))
    }

    fn chain_of(px: Vec<u8>, width: u32, height: u32) -> Vec<MipLevel> {
        build_chain(&Payload::new(px), size(width, height))
    }

    /// The chain descends to 1x1 and every step halves (`CANVAS-065`). Without
    /// the full descent a picture zoomed far out samples the smallest level it
    /// has, which is the crawl the chain exists to remove.
    #[test]
    fn the_chain_halves_down_to_one_texel() {
        let px = vec![128u8; 8 * 8 * 4];
        let chain = chain_of(px, 8, 8);
        let dims: Vec<(u32, u32)> = chain
            .iter()
            .map(|l| (l.size.width.get(), l.size.height.get()))
            .collect();
        assert_eq!(dims, vec![(4, 4), (2, 2), (1, 1)]);
        // The chain omits level 0, so it holds one level less than wgpu counts.
        assert_eq!(chain.len() as u32 + 1, level_count(size(8, 8)).get());
    }

    /// A picture is sampled through its chain: linear inside a level and
    /// between two (`CANVAS-065`).
    ///
    /// A `Nearest` mipmap filter steps visibly between levels, and a sampler
    /// with no minifying filter reads the full-size texels alone.
    #[test]
    fn a_picture_is_sampled_trilinearly() {
        let s = sampler();
        assert_eq!(s.min_filter, wgpu::FilterMode::Linear);
        assert_eq!(s.mag_filter, wgpu::FilterMode::Linear);
        assert_eq!(s.mipmap_filter, wgpu::MipmapFilterMode::Linear);
        assert!(s.lod_max_clamp >= 32.0, "every level stays reachable");
        assert_eq!(s.address_mode_u, wgpu::AddressMode::ClampToEdge);
        assert_eq!(s.address_mode_v, wgpu::AddressMode::ClampToEdge);
    }

    /// A non-square image keeps halving on the axis that still can
    /// (`CANVAS-065`). A chain that stopped at the first axis to reach 1 would
    /// leave the long axis aliasing.
    #[test]
    fn a_flat_image_keeps_halving_its_long_axis() {
        let px = vec![0u8; 8 * 2 * 4];
        let chain = chain_of(px, 8, 2);
        let dims: Vec<(u32, u32)> = chain
            .iter()
            .map(|l| (l.size.width.get(), l.size.height.get()))
            .collect();
        assert_eq!(dims, vec![(4, 1), (2, 1), (1, 1)]);
        assert_eq!(chain.len() as u32 + 1, level_count(size(8, 2)).get());
    }

    /// A reduced texel is the mean of the four it replaces, per channel. This is
    /// the whole point of the chain: a checkerboard must become its average
    /// grey rather than one of its two colours. A nearest reduction, or no
    /// chain at all, leaves one of the two colours on screen.
    #[test]
    fn a_reduced_texel_is_the_mean_of_its_block() {
        // 2x2: black, white, white, black, fully opaque.
        let px: Vec<u8> = vec![
            0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
        ];
        let chain = chain_of(px, 2, 2);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].pixels.as_bytes(), [128, 128, 128, 255]);
    }

    /// An odd dimension carries its last row rather than reading past it
    /// (`CANVAS-065`). The reduction clamps, so a 3-wide image yields a 1-wide
    /// level made of real texels and no out-of-bounds index.
    #[test]
    fn an_odd_dimension_clamps_instead_of_overrunning() {
        let px = vec![200u8; 3 * 4];
        let chain = chain_of(px, 3, 1);
        let dims: Vec<(u32, u32)> = chain
            .iter()
            .map(|l| (l.size.width.get(), l.size.height.get()))
            .collect();
        assert_eq!(dims, vec![(1, 1)]);
        assert_eq!(chain[0].pixels.as_bytes(), [200, 200, 200, 200]);
    }

    /// A source shorter than its declared size yields no chain. The upload path
    /// therefore declines it instead of building levels out of whatever follows
    /// in memory.
    #[test]
    fn a_malformed_source_yields_no_levels() {
        assert!(chain_of(vec![0, 1, 2], 2, 2).is_empty());
        assert!(chain_of(Vec::new(), 0, 0).is_empty());
    }

    /// A 1x1 image has one level and nothing below it, so the chain is empty and
    /// the sampler falls back to level 0.
    #[test]
    fn a_single_texel_image_has_no_chain() {
        assert_eq!(level_count(size(1, 1)).get(), 1);
        assert!(chain_of(vec![1, 2, 3, 4], 1, 1).is_empty());
    }
}
