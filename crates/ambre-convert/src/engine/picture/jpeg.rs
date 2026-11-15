//! The JPEG decoder of the images module: a JPEG file to its samples, scaled
//! and turned (`crate::jpeg`).
//!
//! The module runs `zune-jpeg`, the pure-Rust decoder the `image` crate wraps,
//! without that crate: the scale and the turn are the few lines below. The
//! decode keeps three bytes a pixel, so a file at the 8192 px bound expands to
//! 192 MiB before it is scaled. Only the scaled answer gains its alpha.

use ambre_core::{Image, Payload, Px};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use super::{as_index, exif_orientation, scale, turn};
use crate::images::Pixels;
use crate::jpeg::{JpegDecode, JpegRefusal, JpegTurn, Orientation};

/// The samples of the decode `request` asks for, four bytes a pixel.
///
/// # Errors
///
/// A [`JpegRefusal`] in these cases:
/// * a file that is no JPEG, or a header past the bound;
/// * a size the file cannot give;
/// * a frame that lies about its size;
/// * a header or a scan the decoder refuses.
pub(in crate::engine) fn decode(request: &JpegDecode) -> Result<Pixels, JpegRefusal> {
    let bytes = request.data.as_bytes();
    let stored = crate::jpeg::stored_size(&request.data).ok_or(JpegRefusal::NotJpeg)?;
    let bound = request.bound;
    if stored.width > bound.width || stored.height > bound.height {
        return Err(JpegRefusal::PastBound { stored, bound });
    }
    let size = request.size;
    let zero = Px::<Image>::new(0);
    if size.width == zero
        || size.height == zero
        || size.width > stored.width
        || size.height > stored.height
    {
        return Err(JpegRefusal::SizeUnavailable);
    }
    // The decoder carries the bound too: a frame header that lies, or a
    // second frame, is refused before its samples are allocated.
    let options = DecoderOptions::default()
        .set_strict_mode(false)
        .set_max_width(as_index(bound.width))
        .set_max_height(as_index(bound.height))
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|_| JpegRefusal::Undecodable)?;
    if decoder.dimensions() != Some((as_index(stored.width), as_index(stored.height))) {
        return Err(JpegRefusal::FrameMismatch);
    }
    let orientation = match request.turn {
        JpegTurn::Stored => Orientation::Upright,
        JpegTurn::Upright => decoder
            .exif()
            .map_or(Orientation::Upright, |tiff| exif_orientation(tiff)),
    };
    let samples = decoder.decode().map_err(|_| JpegRefusal::Undecodable)?;
    let rgb = if size == stored {
        samples
    } else {
        let scaled = scale::<3>(&samples, stored, size);
        drop(samples);
        scaled
    };
    let rgba: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, u8::MAX])
        .collect();
    let (rgba, size) = turn(rgba, size, orientation);
    Ok(Pixels {
        size,
        rgba: Payload::new(rgba),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ambre_core::{FileBytes, Size};

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
        assert_eq!(
            decode(&past),
            Err(JpegRefusal::PastBound {
                stored: Size::new(Px::new(64), Px::new(32)),
                bound: past.bound,
            })
        );
        assert_eq!(
            decode(&request(jpeg(64, 32, None), (65, 32), JpegTurn::Stored)),
            Err(JpegRefusal::SizeUnavailable)
        );
        assert_eq!(
            decode(&request(jpeg(64, 32, None), (0, 32), JpegTurn::Stored)),
            Err(JpegRefusal::SizeUnavailable)
        );
        assert_eq!(
            decode(&request(
                b"\x89PNG\r\n\x1a\n".to_vec(),
                (1, 1),
                JpegTurn::Stored
            )),
            Err(JpegRefusal::NotJpeg)
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
        assert!(matches!(
            decode(&request(bytes, (8, 8), JpegTurn::Stored)),
            Err(JpegRefusal::FrameMismatch | JpegRefusal::Undecodable)
        ));
    }
}
