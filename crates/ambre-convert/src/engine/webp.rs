//! The WebP decoder of the images module: a WebP picture to its pixels.
//!
//! The Main instance decodes PNG and JPEG alone, so a WebP decoder there would
//! cost every user its download. The module runs `image-webp`, the pure-Rust
//! decoder the `image` crate itself wraps, and answers RGBA pixels. Toile
//! encodes them as PNG with the encoder Main already links. The `image` crate
//! around the decoder would add about 98 KB of brotli to the module for no
//! pixel more (`ambre-spec-budgets.md section 5`).

use std::io::Cursor;

use ambre_client_core::FileBytes;
use ambre_core::{Image, Payload, Px, Size};
use image_webp::WebPDecoder;

use crate::images::Pixels;
use ambre_core::FileRefusal;

/// The longest edge the module decodes, each edge apart. It mirrors the
/// default image bound of Toile's import limits: a larger picture fails there
/// anyway, and the decoder must not allocate for it first.
pub(super) const MAX_EDGE: Px<Image> = Px::new(8_192);

/// The longest edge of the pixels the module answers. A lossless WebP of
/// [`MAX_EDGE`] a side weighs a few hundred bytes and decodes to 256 MiB,
/// which the Main instance would copy twice. A larger picture is scaled
/// down to it before it leaves the module, as the other decoders answer at
/// the size asked (`P65`, `ambre-spec-budgets.md section 6`).
pub(super) const ANSWER_EDGE: Px<Image> = Px::new(2_048);

/// The size `size` takes once its longest edge fits [`ANSWER_EDGE`], its
/// ratio kept and each edge at least one pixel.
fn answer_size(size: Size<Px<Image>>) -> Size<Px<Image>> {
    let longest = size.width.get().max(size.height.get());
    if longest <= ANSWER_EDGE.get() {
        return size;
    }
    let fit = |edge: u32| {
        let scaled = u64::from(edge) * u64::from(ANSWER_EDGE.get()) / u64::from(longest);
        Px::new(u32::try_from(scaled).unwrap_or(ANSWER_EDGE.get()).max(1))
    };
    Size::new(fit(size.width.get()), fit(size.height.get()))
}

/// The pixels of the WebP picture `file`, four bytes a pixel.
///
/// # Errors
///
/// [`FileRefusal::Malformed`] when the decoder refuses `file`, and
/// [`FileRefusal::PastBound`] when the header declares an edge past
/// [`MAX_EDGE`]. A picture past [`ANSWER_EDGE`] answers scaled down to it.
pub(super) fn decode(file: &Payload<FileBytes>) -> Result<Pixels, FileRefusal> {
    if !crate::is_webp(file) {
        return Err(FileRefusal::OtherDocument);
    }
    let mut decoder =
        WebPDecoder::new(Cursor::new(file.as_bytes())).map_err(|_| FileRefusal::Malformed)?;
    // The header first: a decoded picture takes four bytes per pixel, so the
    // bound is read before any pixel is.
    let (width, height) = decoder.dimensions();
    if width > MAX_EDGE.get() || height > MAX_EDGE.get() {
        return Err(FileRefusal::PastBound);
    }
    let len = decoder.output_buffer_size().ok_or(FileRefusal::PastBound)?;
    let mut samples = vec![0; len];
    decoder
        .read_image(&mut samples)
        .map_err(|_| FileRefusal::Malformed)?;
    // A picture with no alpha decodes three bytes a pixel: each gains an
    // opaque alpha, so every answer has one layout.
    let rgba = if decoder.has_alpha() {
        samples
    } else {
        samples
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, u8::MAX])
            .collect()
    };
    let size = Size::new(Px::new(width), Px::new(height));
    let answer = answer_size(size);
    let rgba = if answer == size {
        rgba
    } else {
        super::picture::scale::<4>(&rgba, size, answer)
    };
    Ok(Pixels {
        size: answer,
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
        let pixels = decode(&Payload::new(small_webp(false))).expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(Px::new(5), Px::new(3)));
        assert_eq!(pixels.rgba.as_bytes().len(), 5 * 3 * 4);
        assert_eq!(at(&pixels, 4, 2), [160, 160, 7, 255]);
        let pixels = decode(&Payload::new(small_webp(true))).expect("the picture decodes");
        assert_eq!(at(&pixels, 1, 1), [40, 80, 7, 200]);
    }

    /// `P65`: a picture past the answer edge leaves the module scaled down
    /// to it, its ratio kept, never at its stored size.
    #[test]
    fn a_large_picture_answers_at_the_answer_edge() {
        let (width, height) = (4_096u32, 64u32);
        let samples = vec![90u8; usize::try_from(width * height * 3).expect("fits")];
        let mut file = Vec::new();
        image_webp::WebPEncoder::new(&mut file)
            .encode(&samples, width, height, image_webp::ColorType::Rgb8)
            .expect("the lossless encoder writes a picture");
        let pixels = decode(&Payload::new(file)).expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(ANSWER_EDGE, Px::new(32)));
        assert_eq!(pixels.rgba.as_bytes().len(), 2_048 * 32 * 4);
    }

    #[test]
    fn the_answer_size_fits_the_edge_and_keeps_its_ratio() {
        let size = |w, h| Size::new(Px::new(w), Px::new(h));
        assert_eq!(answer_size(size(5, 3)), size(5, 3));
        assert_eq!(answer_size(size(8_192, 8_192)), size(2_048, 2_048));
        assert_eq!(answer_size(size(8_192, 1)), size(2_048, 1));
        assert_eq!(answer_size(size(3, 8_192)), size(1, 2_048));
    }

    #[test]
    fn bytes_that_are_no_webp_are_refused() {
        assert!(decode(&Payload::new(b"RIFF\0\0\0\0WEBPVP8 garbage".to_vec())).is_err());
        assert!(decode(&Payload::new(b"\x89PNG\r\n".to_vec())).is_err());
    }
}
