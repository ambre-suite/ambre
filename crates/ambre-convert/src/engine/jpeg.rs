//! The JPEG decoder of the images module: a JPEG file to its samples, scaled
//! and turned (`crate::jpeg`).
//!
//! The module runs `zune-jpeg`, the pure-Rust decoder the `image` crate wraps,
//! without that crate: the scale and the turn are the few lines below. The
//! decode keeps three bytes a pixel, so a file at the 8192 px bound expands to
//! 192 MiB before it is scaled. Only the scaled answer gains its alpha.

use ambre_core::{Image, Payload, Px, Size};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::images::Pixels;
use crate::jpeg::{JpegDecode, JpegTurn};

/// The samples of the decode `request` asks for, four bytes a pixel.
///
/// # Errors
///
/// A refusal in these cases:
/// * a file that is no JPEG, or a header past the bound;
/// * a size the file cannot give;
/// * a frame that lies about its size;
/// * a scan the decoder refuses, in its words.
pub(super) fn decode(request: &JpegDecode) -> Result<Pixels, String> {
    let bytes = request.data.as_bytes();
    let stored =
        crate::jpeg::stored_size(&request.data).ok_or("the file is no JPEG this module reads")?;
    let bound = request.bound;
    if stored.width > bound.width || stored.height > bound.height {
        return Err(format!(
            "the picture is {}x{}, past the {}x{} it may span",
            stored.width.get(),
            stored.height.get(),
            bound.width.get(),
            bound.height.get()
        ));
    }
    let size = request.size;
    let zero = Px::<Image>::new(0);
    if size.width == zero
        || size.height == zero
        || size.width > stored.width
        || size.height > stored.height
    {
        return Err("the size asked for is not one the picture can give".to_owned());
    }
    // The decoder carries the bound too: a frame header that lies, or a
    // second frame, is refused before its samples are allocated.
    let options = DecoderOptions::default()
        .set_strict_mode(false)
        .set_max_width(as_index(bound.width))
        .set_max_height(as_index(bound.height))
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|e| e.to_string())?;
    if decoder.dimensions() != Some((as_index(stored.width), as_index(stored.height))) {
        return Err("the frame the decoder read is not the one the header declares".to_owned());
    }
    let orientation = match request.turn {
        JpegTurn::Stored => Orientation::Upright,
        JpegTurn::Upright => decoder
            .exif()
            .map_or(Orientation::Upright, |tiff| exif_orientation(tiff)),
    };
    let samples = decoder.decode().map_err(|e| e.to_string())?;
    let rgb = if size == stored {
        samples
    } else {
        let scaled = scale(&samples, stored, size);
        drop(samples);
        scaled
    };
    let rgba: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, u8::MAX])
        .collect();
    let (rgba, size) = turn(&rgba, size, orientation);
    Ok(Pixels {
        size,
        rgba: Payload::new(rgba),
    })
}

/// A side as an index into a sample buffer. A side is at most 65 535 pixels,
/// so it always fits.
fn as_index(side: Px<Image>) -> usize {
    usize::try_from(side.get()).unwrap_or(usize::MAX)
}

/// How a picture is turned to stand as it was taken: the eight values of the
/// EXIF orientation tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Orientation {
    Upright,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    /// The orientation of an EXIF value, upright for a value out of range.
    const fn of_exif(value: u16) -> Self {
        match value {
            2 => Self::FlipHorizontal,
            3 => Self::Rotate180,
            4 => Self::FlipVertical,
            5 => Self::Transpose,
            6 => Self::Rotate90,
            7 => Self::Transverse,
            8 => Self::Rotate270,
            _ => Self::Upright,
        }
    }

    /// Whether the turn swaps the two sides.
    const fn swaps_sides(self) -> bool {
        matches!(
            self,
            Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270
        )
    }
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

/// `rgb` of `from` scaled down to `to`, three bytes a pixel, through a
/// triangle filter as wide as the scale: the filter `image` names `Triangle`.
/// The rows are scaled first, so the pass between holds `to.width` columns.
fn scale(rgb: &[u8], from: Size<Px<Image>>, to: Size<Px<Image>>) -> Vec<u8> {
    let (sw, sh) = (as_index(from.width), as_index(from.height));
    let (dw, dh) = (as_index(to.width), as_index(to.height));
    let across = weights(sw, dw);
    let mut rows = vec![0u8; dw * sh * 3];
    for (y, row) in rows.chunks_exact_mut(dw * 3).enumerate() {
        let src = rgb.get(y * sw * 3..(y + 1) * sw * 3).unwrap_or_default();
        for (x, out) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let (first, taps) = across.get(x).map_or((0, &[][..]), |(f, t)| (*f, t));
            blend(out, taps, |k| src.get((first + k) * 3..(first + k) * 3 + 3));
        }
    }
    let down = weights(sh, dh);
    let mut out = vec![0u8; dw * dh * 3];
    for (y, line) in out.chunks_exact_mut(dw * 3).enumerate() {
        let (first, taps) = down.get(y).map_or((0, &[][..]), |(f, t)| (*f, t));
        for (x, pixel) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            blend(pixel, taps, |k| {
                let at = ((first + k) * dw + x) * 3;
                rows.get(at..at + 3)
            });
        }
    }
    out
}

/// One output pixel: the weighted sum of the source pixels `taps` names.
fn blend<'a>(out: &mut [u8], taps: &[f32], pixel: impl Fn(usize) -> Option<&'a [u8]>) {
    let mut sum = [0f32; 3];
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

/// For each of `dst` output positions, the first source position it reads
/// and the normalised weights from there, when `src` positions scale down to
/// `dst`.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positions under 65 536, exact in an f32, and clamped before the casts back"
)]
fn weights(src: usize, dst: usize) -> Vec<(usize, Vec<f32>)> {
    let ratio = src as f32 / dst.max(1) as f32;
    let support = ratio.max(1.0);
    (0..dst)
        .map(|i| {
            let centre = (i as f32 + 0.5) * ratio;
            let first = (centre - support).floor().max(0.0) as usize;
            let last = ((centre + support).ceil() as usize).min(src);
            let raw: Vec<f32> = (first..last)
                .map(|j| (1.0 - ((j as f32 + 0.5 - centre) / support).abs()).max(0.0))
                .collect();
            let total: f32 = raw.iter().sum();
            let norm = if total > 0.0 { total } else { 1.0 };
            (first, raw.into_iter().map(|w| w / norm).collect())
        })
        .collect()
}

/// `rgba` of `size` turned by `orientation`, and its size after the turn.
fn turn(
    rgba: &[u8],
    size: Size<Px<Image>>,
    orientation: Orientation,
) -> (Vec<u8>, Size<Px<Image>>) {
    if orientation == Orientation::Upright {
        return (rgba.to_vec(), size);
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
pub(crate) mod tests {
    use super::*;
    use ambre_core::FileBytes;

    /// A JPEG of `width` by `height`, which the `image` encoder writes at
    /// test time. Its left half is red and its right half blue, so a turn
    /// shows. With `orientation`, an EXIF segment states it.
    pub(crate) fn jpeg(width: u32, height: u32, orientation: Option<u8>) -> Vec<u8> {
        let picture = image::RgbImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                image::Rgb([250, 0, 0])
            } else {
                image::Rgb([0, 0, 250])
            }
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
            .encode_image(&picture)
            .expect("the encoder writes a picture");
        match orientation {
            None => out,
            Some(value) => {
                let mut tagged = vec![0xFF, 0xD8];
                tagged.extend_from_slice(&[
                    0xFF, 0xE1, 0x00, 0x22, b'E', b'x', b'i', b'f', 0, 0, b'M', b'M', 0x00, 0x2A,
                    0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00,
                    0x01, 0x00, value, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ]);
                tagged.extend_from_slice(out.get(2..).unwrap_or_default());
                tagged
            }
        }
    }

    fn request(data: Vec<u8>, size: (u32, u32), turn: JpegTurn) -> JpegDecode {
        JpegDecode {
            data: Payload::<FileBytes>::new(data),
            bound: Size::new(Px::new(8192), Px::new(8192)),
            size: Size::new(Px::new(size.0), Px::new(size.1)),
            turn,
        }
    }

    fn at(pixels: &Pixels, x: usize, y: usize) -> [u8; 4] {
        let w = as_index(pixels.size.width);
        let s = pixels.rgba.as_bytes();
        let i = (y * w + x) * 4;
        [s[i], s[i + 1], s[i + 2], s[i + 3]]
    }

    fn reddish(p: [u8; 4]) -> bool {
        p[0] > 180 && p[2] < 80 && p[3] == 255
    }

    fn bluish(p: [u8; 4]) -> bool {
        p[2] > 180 && p[0] < 80 && p[3] == 255
    }

    #[test]
    fn a_jpeg_decodes_to_its_rgba_samples_at_its_size() {
        let pixels = decode(&request(jpeg(64, 32, None), (64, 32), JpegTurn::Stored))
            .expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(Px::new(64), Px::new(32)));
        assert_eq!(pixels.rgba.as_bytes().len(), 64 * 32 * 4);
        assert!(reddish(at(&pixels, 4, 16)));
        assert!(bluish(at(&pixels, 60, 16)));
    }

    #[test]
    fn a_jpeg_scales_down_to_the_size_asked_its_halves_kept() {
        let pixels = decode(&request(jpeg(400, 200, None), (100, 50), JpegTurn::Stored))
            .expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(Px::new(100), Px::new(50)));
        assert_eq!(pixels.rgba.as_bytes().len(), 100 * 50 * 4);
        assert!(reddish(at(&pixels, 10, 25)));
        assert!(bluish(at(&pixels, 90, 25)));
    }

    #[test]
    fn an_upright_decode_turns_the_picture_as_its_exif_states() {
        // Rotate90: the red left half lands on top, and the sides swap.
        let pixels = decode(&request(jpeg(64, 32, Some(6)), (64, 32), JpegTurn::Upright))
            .expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(Px::new(32), Px::new(64)));
        assert!(reddish(at(&pixels, 16, 4)));
        assert!(bluish(at(&pixels, 16, 60)));
        // The stored samples ignore the tag.
        let stored = decode(&request(jpeg(64, 32, Some(6)), (64, 32), JpegTurn::Stored))
            .expect("the picture decodes");
        assert_eq!(stored.size, Size::new(Px::new(64), Px::new(32)));
        // Rotate270: the red half lands at the bottom.
        let pixels = decode(&request(jpeg(64, 32, Some(8)), (64, 32), JpegTurn::Upright))
            .expect("the picture decodes");
        assert!(bluish(at(&pixels, 16, 4)));
        assert!(reddish(at(&pixels, 16, 60)));
        // FlipHorizontal: the halves trade places, the size kept.
        let pixels = decode(&request(jpeg(64, 32, Some(2)), (64, 32), JpegTurn::Upright))
            .expect("the picture decodes");
        assert!(bluish(at(&pixels, 4, 16)));
        assert!(reddish(at(&pixels, 60, 16)));
    }

    #[test]
    fn a_header_past_the_bound_or_a_size_the_file_cannot_give_is_refused() {
        let mut past = request(jpeg(64, 32, None), (64, 32), JpegTurn::Stored);
        past.bound = Size::new(Px::new(63), Px::new(8192));
        assert!(decode(&past).is_err());
        assert!(decode(&request(jpeg(64, 32, None), (65, 32), JpegTurn::Stored)).is_err());
        assert!(decode(&request(jpeg(64, 32, None), (0, 32), JpegTurn::Stored)).is_err());
        assert!(
            decode(&request(
                b"\x89PNG\r\n\x1a\n".to_vec(),
                (1, 1),
                JpegTurn::Stored
            ))
            .is_err()
        );
        let mut cut = jpeg(64, 32, None);
        cut.truncate(cut.len() / 3);
        // A scan cut short decodes what it holds or is refused, and never
        // answers samples of another size.
        if let Ok(pixels) = decode(&request(cut, (64, 32), JpegTurn::Stored)) {
            assert_eq!(pixels.rgba.as_bytes().len(), 64 * 32 * 4);
        }
    }

    #[test]
    fn a_frame_that_lies_about_its_size_is_refused() {
        // The header the walk reads says 64x32, and a second frame says more.
        let mut bytes = jpeg(64, 32, None);
        let sof = bytes
            .windows(2)
            .position(|w| w == [0xFF, 0xC0])
            .expect("a baseline frame");
        // Make the walk meet a fake frame of 8x8 first.
        let fake = [
            0xFF, 0xC1, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00,
        ];
        bytes.splice(sof..sof, fake);
        assert!(decode(&request(bytes, (8, 8), JpegTurn::Stored)).is_err());
    }

    #[test]
    fn the_triangle_scale_keeps_a_flat_colour_flat() {
        let from = Size::new(Px::new(7), Px::new(5));
        let flat = vec![123u8; 7 * 5 * 3];
        let out = scale(&flat, from, Size::new(Px::new(3), Px::new(2)));
        assert_eq!(out.len(), 3 * 2 * 3);
        assert!(out.iter().all(|&b| b == 123));
    }
}
