//! The WebP decoder of the images module: a WebP picture to its pixels.
//!
//! The Main instance decodes PNG and JPEG alone, so a WebP decoder there would
//! cost every user its download. The module runs `image-webp`, the pure-Rust
//! decoder the `image` crate itself wraps, and answers RGBA pixels. Toile
//! encodes them as PNG with the encoder Main already links. The `image` crate
//! around the decoder would add about 98 KB of brotli to the module for no
//! pixel more (`ambre-spec-budgets.md section 5`).

use std::io::Cursor;

use ambre_core::{Payload, Px, Size};
use image_webp::WebPDecoder;

use crate::images::Pixels;

/// The longest edge the module decodes, each edge apart. It mirrors the
/// default image bound of Toile's import limits: a larger picture fails there
/// anyway, and the decoder must not allocate for it first.
const MAX_EDGE: u32 = 8_192;

/// The pixels of the WebP picture `bytes`, four bytes a pixel.
///
/// # Errors
///
/// The words of the decoder when it refuses `bytes`, and a refusal when the
/// header declares an edge past [`MAX_EDGE`].
pub(super) fn decode(bytes: &[u8]) -> Result<Pixels, String> {
    if !crate::is_webp(bytes) {
        return Err("the picture is not a WebP file".to_owned());
    }
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    // The header first: a decoded picture takes four bytes per pixel, so the
    // bound is read before any pixel is.
    let (width, height) = decoder.dimensions();
    if width > MAX_EDGE || height > MAX_EDGE {
        return Err(format!(
            "the picture is {width}x{height}, past the {MAX_EDGE} px a picture may span"
        ));
    }
    let len = decoder
        .output_buffer_size()
        .ok_or_else(|| "the picture is too large to decode".to_owned())?;
    let mut samples = vec![0; len];
    decoder
        .read_image(&mut samples)
        .map_err(|e| e.to_string())?;
    // A picture with no alpha decodes three bytes a pixel: each gains an
    // opaque alpha, so every answer has one layout.
    let rgba = if decoder.has_alpha() {
        samples
    } else {
        samples
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], u8::MAX])
            .collect()
    };
    Ok(Pixels {
        size: Size::new(Px::new(width), Px::new(height)),
        rgba: Payload::new(rgba),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 5 by 3 WebP picture, written by the lossless encoder of `image-webp`
    /// at test time: the fixture is generated, never downloaded.
    pub(crate) fn small_webp(alpha: bool) -> Vec<u8> {
        let (width, height) = (5u32, 3u32);
        let pixel = |x: u32, y: u32| {
            [
                u8::try_from(x * 40).unwrap_or(0),
                u8::try_from(y * 80).unwrap_or(0),
                7,
            ]
        };
        let mut out = Vec::new();
        let encoder = image_webp::WebPEncoder::new(&mut out);
        let (samples, color): (Vec<u8>, _) = if alpha {
            (
                (0..height)
                    .flat_map(|y| {
                        (0..width).flat_map(move |x| {
                            let [r, g, b] = pixel(x, y);
                            [r, g, b, 200]
                        })
                    })
                    .collect(),
                image_webp::ColorType::Rgba8,
            )
        } else {
            (
                (0..height)
                    .flat_map(|y| (0..width).flat_map(move |x| pixel(x, y)))
                    .collect(),
                image_webp::ColorType::Rgb8,
            )
        };
        encoder
            .encode(&samples, width, height, color)
            .expect("the lossless encoder writes a picture");
        out
    }

    fn at(pixels: &Pixels, x: usize, y: usize) -> [u8; 4] {
        let i = (y * 5 + x) * 4;
        let s = pixels.rgba.as_bytes();
        [s[i], s[i + 1], s[i + 2], s[i + 3]]
    }

    #[test]
    fn a_webp_picture_decodes_to_its_rgba_pixels() {
        let pixels = decode(&small_webp(false)).expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(Px::new(5), Px::new(3)));
        assert_eq!(pixels.rgba.as_bytes().len(), 5 * 3 * 4);
        assert_eq!(at(&pixels, 4, 2), [160, 160, 7, 255]);
        let pixels = decode(&small_webp(true)).expect("the picture decodes");
        assert_eq!(at(&pixels, 1, 1), [40, 80, 7, 200]);
    }

    #[test]
    fn bytes_that_are_no_webp_are_refused() {
        assert!(decode(b"RIFF\0\0\0\0WEBPVP8 garbage").is_err());
        assert!(decode(b"\x89PNG\r\n").is_err());
    }
}
