//! The picture decoders of the images module, and the pixel work they share:
//! the scale, the turn and the EXIF orientation reader (`crate::jpeg`,
//! `crate::raster`).
//!
//! Each decoder is a child module, so the shared helpers stay private to
//! this module and its children.

use ambre_core::{Image, Px, Size};

use crate::jpeg::Orientation;

pub(in crate::engine) mod jpeg;
#[cfg(feature = "images")]
pub(in crate::engine) mod raster;

/// A side as an index into a sample buffer. A side is at most 65 535 pixels,
/// so it always fits.
fn as_index(side: Px<Image>) -> usize {
    usize::try_from(side.get()).unwrap_or(usize::MAX)
}

/// The orientation tag of the first image file directory of `tiff`, the EXIF
/// payload from its TIFF header on. Upright when the tag is absent or the
/// bytes do not read.
fn exif_orientation(tiff: &[u8]) -> Orientation {
    const TAG: u16 = 0x0112;
    let read16 = |at: usize, big: bool| -> Option<u16> {
        let pair: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if big {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        })
    };
    let found = || -> Option<u16> {
        let big = match tiff.get(..2)? {
            b"MM" => true,
            b"II" => false,
            _ => return None,
        };
        let quad: [u8; 4] = tiff.get(4..8)?.try_into().ok()?;
        let ifd = usize::try_from(if big {
            u32::from_be_bytes(quad)
        } else {
            u32::from_le_bytes(quad)
        })
        .ok()?;
        let entries = read16(ifd, big)?;
        (0..usize::from(entries))
            .map(|i| ifd + 2 + i * 12)
            .find(|&entry| read16(entry, big) == Some(TAG))
            .and_then(|entry| read16(entry + 8, big))
    };
    found().map_or(Orientation::Upright, Orientation::of_exif)
}

/// `samples` of `from` scaled down to `to`, `C` bytes a pixel, through a
/// triangle filter as wide as the scale: the filter `image` names `Triangle`.
/// Each channel scales apart, alpha included, as `image` scales RGBA. The
/// rows are scaled first, so the pass between holds `to.width` columns.
fn scale<const C: usize>(samples: &[u8], from: Size<Px<Image>>, to: Size<Px<Image>>) -> Vec<u8> {
    let (sw, sh) = (as_index(from.width), as_index(from.height));
    let (dw, dh) = (as_index(to.width), as_index(to.height));
    let (across_first, across) = weights(sw, dw);
    let across_taps = across.len() / dw.max(1);
    let mut rows = vec![0u8; dw * sh * C];
    for (y, row) in rows.chunks_exact_mut(dw * C).enumerate() {
        let src = samples
            .get(y * sw * C..(y + 1) * sw * C)
            .unwrap_or_default();
        for (x, out) in row.as_chunks_mut::<C>().0.iter_mut().enumerate() {
            let first = across_first.get(x).copied().unwrap_or(0);
            let taps = across
                .get(x * across_taps..(x + 1) * across_taps)
                .unwrap_or_default();
            blend::<C>(out, taps, |k| src.get((first + k) * C..(first + k) * C + C));
        }
    }
    let (down_first, down) = weights(sh, dh);
    let down_taps = down.len() / dh.max(1);
    let mut out = vec![0u8; dw * dh * C];
    for (y, line) in out.chunks_exact_mut(dw * C).enumerate() {
        let first = down_first.get(y).copied().unwrap_or(0);
        let taps = down
            .get(y * down_taps..(y + 1) * down_taps)
            .unwrap_or_default();
        for (x, pixel) in line.as_chunks_mut::<C>().0.iter_mut().enumerate() {
            blend::<C>(pixel, taps, |k| {
                let at = ((first + k) * dw + x) * C;
                rows.get(at..at + C)
            });
        }
    }
    out
}

/// One output pixel: the weighted sum of the source pixels `taps` names.
fn blend<'a, const C: usize>(
    out: &mut [u8],
    taps: &[f32],
    pixel: impl Fn(usize) -> Option<&'a [u8]>,
) {
    let mut sum = [0f32; C];
    for (k, w) in taps.iter().enumerate() {
        if let Some(p) = pixel(k) {
            for (s, v) in sum.iter_mut().zip(p) {
                *s += f32::from(*v) * w;
            }
        }
    }
    for (o, s) in out.iter_mut().zip(sum) {
        // A weighted mean of bytes stays in their range once clamped.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to the range of a byte"
        )]
        let byte = s.round().clamp(0.0, 255.0) as u8;
        *o = byte;
    }
}

/// For each of `dst` output positions, the first source position it reads,
/// and the normalised weights from there, when `src` positions scale down to
/// `dst`. The weights lie in one buffer, the same number for each output
/// position: the widest the filter reaches, padded with zeros. So the scale
/// allocates twice, not once per position.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positions under 65 536, exact in an f32, and clamped before the casts back"
)]
fn weights(src: usize, dst: usize) -> (Vec<usize>, Vec<f32>) {
    let ratio = src as f32 / dst.max(1) as f32;
    let support = ratio.max(1.0);
    // The span from the floor of `centre - support` to the ceiling of
    // `centre + support` holds at most this many positions.
    let taps = (2.0 * support).ceil() as usize + 1;
    let mut firsts = Vec::with_capacity(dst);
    let mut flat = vec![0f32; dst * taps];
    for (i, row) in flat.chunks_exact_mut(taps).enumerate() {
        let centre = (i as f32 + 0.5) * ratio;
        let first = (centre - support).floor().max(0.0) as usize;
        let last = ((centre + support).ceil() as usize)
            .min(src)
            .min(first + taps);
        for (w, j) in row.iter_mut().zip(first..last) {
            *w = (1.0 - ((j as f32 + 0.5 - centre) / support).abs()).max(0.0);
        }
        let total: f32 = row.iter().sum();
        let norm = if total > 0.0 { total } else { 1.0 };
        row.iter_mut().for_each(|w| *w /= norm);
        firsts.push(first);
    }
    (firsts, flat)
}

/// `rgba` of `size` turned by `orientation`, and its size after the turn.
/// The samples move in, so an upright picture keeps its buffer.
fn turn(
    rgba: Vec<u8>,
    size: Size<Px<Image>>,
    orientation: Orientation,
) -> (Vec<u8>, Size<Px<Image>>) {
    if orientation == Orientation::Upright {
        return (rgba, size);
    }
    let (w, h) = (as_index(size.width), as_index(size.height));
    let out_size = if orientation.swaps_sides() {
        Size::new(size.height, size.width)
    } else {
        size
    };
    let ow = as_index(out_size.width);
    let mut out = vec![0u8; rgba.len()];
    for (i, pixel) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % ow, i / ow);
        // The source pixel each output pixel takes.
        let (sx, sy) = match orientation {
            Orientation::Upright => (x, y),
            Orientation::FlipHorizontal => (w - 1 - x, y),
            Orientation::Rotate180 => (w - 1 - x, h - 1 - y),
            Orientation::FlipVertical => (x, h - 1 - y),
            Orientation::Transpose => (y, x),
            Orientation::Rotate90 => (y, h - 1 - x),
            Orientation::Transverse => (w - 1 - y, h - 1 - x),
            Orientation::Rotate270 => (w - 1 - y, x),
        };
        let at = (sy * w + sx) * 4;
        if let Some(src) = rgba.get(at..at + 4) {
            pixel.copy_from_slice(src);
        }
    }
    (out, out_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_output_position_weighs_one_in_all() {
        for (src, dst) in [(7, 3), (8192, 256), (300, 299), (5, 5)] {
            let (firsts, flat) = weights(src, dst);
            assert_eq!(firsts.len(), dst);
            let taps = flat.len() / dst;
            for (i, row) in flat.chunks_exact(taps).enumerate() {
                let sum: f32 = row.iter().sum();
                assert!((sum - 1.0).abs() < 1e-4, "{src}->{dst} at {i}: {sum}");
                assert!(firsts[i] < src);
            }
        }
    }

    #[test]
    fn the_triangle_scale_keeps_a_flat_colour_flat() {
        let from = Size::new(Px::new(7), Px::new(5));
        let flat = vec![123u8; 7 * 5 * 3];
        let out = scale::<3>(&flat, from, Size::new(Px::new(3), Px::new(2)));
        assert_eq!(out.len(), 3 * 2 * 3);
        assert!(out.iter().all(|&b| b == 123));
    }
}
