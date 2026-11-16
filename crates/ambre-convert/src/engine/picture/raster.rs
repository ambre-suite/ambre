//! The PNG and GIF decoders of the images module: a file to its samples,
//! scaled, and turned for a PNG (`crate::raster`).
//!
//! The module runs `png` and `gif`, the pure-Rust decoders the `image` crate
//! wraps, without that crate. The scale and the turn are those of the JPEG
//! decode, over four bytes a pixel. A GIF frame is composed onto the logical
//! screen as its disposal states, so each frame of the answer is whole.

use std::io::Cursor;
use std::num::NonZeroU64;

use ambre_core::{Count, Image, Ms, Octets, Payload, Px, Size, TimeSpan};

use super::{as_index, exif_orientation, scale, turn};
use crate::images::{Frame, Frames, Pixels};
use crate::jpeg::{JpegTurn, Orientation};
use crate::raster::{GifDecode, PngDecode, RasterRefusal};

/// The bytes a decoder may hold past the samples of a picture at the bound:
/// its line buffers and the window of its inflate.
const DECODER_SLACK: Count<Octets> = Count::new(16 * 1024 * 1024);

/// The samples of the PNG decode `request` asks for, four bytes a pixel.
///
/// # Errors
///
/// A [`RasterRefusal`] in these cases:
/// * a file that is no PNG, or a header past the bound;
/// * a size the file cannot give;
/// * a decoder that reads another size than the header;
/// * a file the decoder refuses.
pub(in crate::engine) fn decode_png(request: &PngDecode) -> Result<Pixels, RasterRefusal> {
    let stored = crate::raster::png_size(&request.data).ok_or(RasterRefusal::OtherFormat)?;
    held(stored, request.bound, request.size)?;
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(request.data.as_bytes()),
        png::Limits {
            bytes: usize::try_from(alloc_bound(request.bound).get()).unwrap_or(usize::MAX),
        },
    );
    // Every colour type and depth reads as eight-bit samples: a palette
    // expands, sixteen bits strip to eight.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|_| RasterRefusal::Undecodable)?;
    let (width, height) = reader.info().size();
    if Size::new(Px::new(width), Px::new(height)) != stored {
        return Err(RasterRefusal::FrameMismatch);
    }
    let orientation = match request.turn {
        JpegTurn::Stored => Orientation::Upright,
        JpegTurn::Upright => reader
            .info()
            .exif_metadata
            .as_deref()
            .map_or(Orientation::Upright, exif_orientation),
    };
    let len = reader
        .output_buffer_size()
        .ok_or(RasterRefusal::Undecodable)?;
    let mut samples = vec![0u8; len];
    let frame = reader
        .next_frame(&mut samples)
        .map_err(|_| RasterRefusal::Undecodable)?;
    samples.truncate(frame.buffer_size());
    let rgba = to_rgba(samples, frame.color_type);
    let rgba = if request.size == stored {
        rgba
    } else {
        scale::<4>(&rgba, stored, request.size)
    };
    let (rgba, size) = turn(rgba, request.size, orientation);
    Ok(Pixels {
        size,
        rgba: Payload::new(rgba),
    })
}

/// Eight-bit samples of `color` as four bytes a pixel, an opaque alpha added
/// where the file stores none.
fn to_rgba(samples: Vec<u8>, color: png::ColorType) -> Vec<u8> {
    match color {
        png::ColorType::Rgba => samples,
        png::ColorType::Rgb => samples
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, u8::MAX])
            .collect(),
        png::ColorType::GrayscaleAlpha => samples
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[l, a]| [l, l, l, a])
            .collect(),
        // A palette expands to RGB or RGBA under the transformation above,
        // so this arm reads grey alone.
        png::ColorType::Grayscale | png::ColorType::Indexed => {
            samples.iter().flat_map(|&l| [l, l, l, u8::MAX]).collect()
        }
    }
}

/// The frames of the GIF decode `request` asks for, each composed onto the
/// logical screen and scaled to the size asked.
///
/// A broken frame after the first ends the animation: the frames before it
/// play. The frames after the first stop at the byte budget of the request.
///
/// # Errors
///
/// A [`RasterRefusal`] in these cases:
/// * a file that is no GIF, or a logical screen past the bound;
/// * a size the file cannot give;
/// * a decoder that reads another screen than the header;
/// * a file whose first frame the decoder refuses.
pub(in crate::engine) fn decode_gif(request: &GifDecode) -> Result<Frames, RasterRefusal> {
    let stored = crate::raster::gif_size(&request.data).ok_or(RasterRefusal::OtherFormat)?;
    held(stored, request.bound, request.size)?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    if let Some(limit) = NonZeroU64::new(alloc_bound(request.bound).get()) {
        options.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    }
    let mut decoder = options
        .read_info(Cursor::new(request.data.as_bytes()))
        .map_err(|_| RasterRefusal::Undecodable)?;
    let screen = Size::new(
        Px::new(u32::from(decoder.width())),
        Px::new(u32::from(decoder.height())),
    );
    if screen != stored {
        return Err(RasterRefusal::FrameMismatch);
    }
    let size = request.size;
    let frame_bytes = as_index(size.width) * as_index(size.height) * 4;
    // The first frame always, then as many as the budget holds.
    let budget = usize::try_from(request.budget.get()).unwrap_or(usize::MAX);
    let most = 1 + budget.saturating_sub(frame_bytes) / frame_bytes.max(1);
    let mut canvas = Canvas::new(stored);
    let mut frames = Vec::new();
    while frames.len() < most {
        let frame = match decoder.read_next_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(_) if frames.is_empty() => return Err(RasterRefusal::Undecodable),
            Err(_) => break,
        };
        let delay = TimeSpan::<Ms>::new(u64::from(frame.delay) * 10);
        let rgba = canvas.compose(frame);
        let rgba = if size == stored {
            rgba
        } else {
            scale::<4>(&rgba, stored, size)
        };
        frames.push(Frame {
            rgba: Payload::new(rgba),
            delay,
        });
    }
    if frames.is_empty() {
        return Err(RasterRefusal::Undecodable);
    }
    Ok(Frames { size, frames })
}

/// The logical screen of a GIF, onto which each frame draws. It keeps what
/// a frame's disposal leaves for the next one.
struct Canvas {
    rgba: Vec<u8>,
    size: Size<Px<Image>>,
}

impl Canvas {
    fn new(size: Size<Px<Image>>) -> Self {
        Self {
            rgba: vec![0u8; as_index(size.width) * as_index(size.height) * 4],
            size,
        }
    }

    /// Draw `frame` over the screen and answer the screen as it shows, then
    /// apply the frame's disposal. A transparent sample keeps what lies
    /// under it, and a frame past the screen is clipped to it.
    fn compose(&mut self, frame: &gif::Frame<'_>) -> Vec<u8> {
        let (width, height) = (as_index(self.size.width), as_index(self.size.height));
        let (left, top) = (usize::from(frame.left), usize::from(frame.top));
        let (fw, fh) = (usize::from(frame.width), usize::from(frame.height));
        let before = (frame.dispose == gif::DisposalMethod::Previous).then(|| self.rgba.clone());
        for (y, row) in frame
            .buffer
            .chunks_exact(fw.max(1) * 4)
            .take(fh)
            .enumerate()
        {
            let at = top + y;
            if at >= height {
                break;
            }
            for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
                let across = left + x;
                if across >= width {
                    break;
                }
                if pixel[3] == 0 {
                    continue;
                }
                let i = (at * width + across) * 4;
                if let Some(out) = self.rgba.get_mut(i..i + 4) {
                    out.copy_from_slice(pixel);
                }
            }
        }
        let shown = self.rgba.clone();
        match frame.dispose {
            gif::DisposalMethod::Background => self.clear(frame),
            gif::DisposalMethod::Previous => {
                if let Some(before) = before {
                    self.rgba = before;
                }
            }
            gif::DisposalMethod::Keep | gif::DisposalMethod::Any => {}
        }
        shown
    }

    /// Clear the rectangle a frame drew to transparent, as browsers dispose
    /// to the background.
    fn clear(&mut self, frame: &gif::Frame<'_>) {
        let (width, height) = (as_index(self.size.width), as_index(self.size.height));
        let (left, top) = (usize::from(frame.left), usize::from(frame.top));
        let right = (left + usize::from(frame.width)).min(width);
        for y in top..(top + usize::from(frame.height)).min(height) {
            let row = y * width;
            if let Some(span) = self.rgba.get_mut((row + left) * 4..(row + right) * 4) {
                span.fill(0);
            }
        }
    }
}

/// Hold the stored size of a file to the bound of its request, and the size
/// asked to the stored size.
fn held(
    stored: Size<Px<Image>>,
    bound: Size<Px<Image>>,
    size: Size<Px<Image>>,
) -> Result<(), RasterRefusal> {
    if stored.width > bound.width || stored.height > bound.height {
        return Err(RasterRefusal::PastBound { stored, bound });
    }
    let zero = Px::<Image>::new(0);
    if size.width == zero
        || size.height == zero
        || size.width > stored.width
        || size.height > stored.height
    {
        return Err(RasterRefusal::SizeUnavailable);
    }
    Ok(())
}

/// The most a decoder may allocate for a picture at `bound`: its samples at
/// four bytes a pixel, and the slack of the decoder.
fn alloc_bound(bound: Size<Px<Image>>) -> Count<Octets> {
    Count::new(
        (u64::from(bound.width.get()) * u64::from(bound.height.get()))
            .saturating_mul(4)
            .saturating_add(DECODER_SLACK.get()),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ambre_client_core::FileBytes;
    use image::codecs::gif::{GifEncoder, Repeat};

    /// A PNG of `width` by `height`, which the `image` encoder writes at
    /// test time. Its left half is red and its right half blue, so a turn
    /// shows.
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let picture = image::RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                image::Rgba([250, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 250, 255])
            }
        });
        let mut out = Vec::new();
        picture
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("the encoder writes a picture");
        out
    }

    /// `png` with an `eXIf` chunk that states `orientation`, before `IDAT`.
    fn png_turned(width: u32, height: u32, orientation: u8) -> Vec<u8> {
        let plain = png(width, height);
        // The signature and the IHDR chunk: 8 + 4 + 4 + 13 + 4 bytes.
        let (head, tail) = plain.split_at(33);
        let exif = [
            b'M',
            b'M',
            0x00,
            0x2A,
            0x00,
            0x00,
            0x00,
            0x08,
            0x00,
            0x01,
            0x01,
            0x12,
            0x00,
            0x03,
            0x00,
            0x00,
            0x00,
            0x01,
            0x00,
            orientation,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
        ];
        let mut chunk = b"eXIf".to_vec();
        chunk.extend_from_slice(&exif);
        let crc = crc32(&chunk);
        let mut out = head.to_vec();
        out.extend_from_slice(&u32::try_from(exif.len()).expect("short").to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc.to_be_bytes());
        out.extend_from_slice(tail);
        out
    }

    /// The CRC-32 of a PNG chunk, over its type and its data.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A GIF of `count` frames of `width` by `height`, each a flat shade of
    /// red, each showing `delay_ms`.
    pub(crate) fn gif(count: u8, width: u32, height: u32, delay_ms: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            encoder.set_repeat(Repeat::Infinite).expect("repeat");
            for i in 0..count {
                let buffer =
                    image::RgbaImage::from_pixel(width, height, image::Rgba([40 * i, 0, 0, 255]));
                encoder
                    .encode_frame(image::Frame::from_parts(
                        buffer,
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(delay_ms, 1),
                    ))
                    .expect("the encoder writes a frame");
            }
        }
        out
    }

    fn png_request(data: Vec<u8>, size: (u32, u32), turn: JpegTurn) -> PngDecode {
        PngDecode {
            data: Payload::<FileBytes>::new(data),
            bound: Size::new(Px::new(8192), Px::new(8192)),
            size: Size::new(Px::new(size.0), Px::new(size.1)),
            turn,
        }
    }

    fn gif_request(data: Vec<u8>, size: (u32, u32), budget: usize) -> GifDecode {
        GifDecode {
            data: Payload::<FileBytes>::new(data),
            bound: Size::new(Px::new(4096), Px::new(4096)),
            size: Size::new(Px::new(size.0), Px::new(size.1)),
            budget: Count::of(budget),
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
    fn a_png_decodes_to_its_rgba_samples_and_scales_down_its_halves_kept() {
        let pixels = decode_png(&png_request(png(64, 32), (64, 32), JpegTurn::Stored))
            .expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(Px::new(64), Px::new(32)));
        assert_eq!(pixels.rgba.as_bytes().len(), 64 * 32 * 4);
        assert!(reddish(at(&pixels, 4, 16)));
        assert!(bluish(at(&pixels, 60, 16)));
        let small = decode_png(&png_request(png(400, 200), (100, 50), JpegTurn::Stored))
            .expect("the picture decodes");
        assert_eq!(small.size, Size::new(Px::new(100), Px::new(50)));
        assert_eq!(small.rgba.as_bytes().len(), 100 * 50 * 4);
        assert!(reddish(at(&small, 10, 25)));
        assert!(bluish(at(&small, 90, 25)));
    }

    #[test]
    fn a_grey_or_a_palette_png_decodes_to_rgba() {
        let mut grey = Vec::new();
        image::GrayImage::from_pixel(5, 4, image::Luma([77]))
            .write_to(&mut Cursor::new(&mut grey), image::ImageFormat::Png)
            .expect("the encoder writes a picture");
        let pixels =
            decode_png(&png_request(grey, (5, 4), JpegTurn::Stored)).expect("the picture decodes");
        assert_eq!(at(&pixels, 2, 2), [77, 77, 77, 255]);
        let mut rgb = Vec::new();
        image::RgbImage::from_pixel(3, 2, image::Rgb([1, 2, 3]))
            .write_to(&mut Cursor::new(&mut rgb), image::ImageFormat::Png)
            .expect("the encoder writes a picture");
        let pixels =
            decode_png(&png_request(rgb, (3, 2), JpegTurn::Stored)).expect("the picture decodes");
        assert_eq!(at(&pixels, 1, 1), [1, 2, 3, 255]);
    }

    #[test]
    fn an_upright_png_decode_turns_the_picture_as_its_exif_chunk_states() {
        // Rotate90: the red left half lands on top, and the sides swap.
        let pixels = decode_png(&png_request(
            png_turned(64, 32, 6),
            (64, 32),
            JpegTurn::Upright,
        ))
        .expect("the picture decodes");
        assert_eq!(pixels.size, Size::new(Px::new(32), Px::new(64)));
        assert!(reddish(at(&pixels, 16, 4)));
        assert!(bluish(at(&pixels, 16, 60)));
        let stored = decode_png(&png_request(
            png_turned(64, 32, 6),
            (64, 32),
            JpegTurn::Stored,
        ))
        .expect("the picture decodes");
        assert_eq!(stored.size, Size::new(Px::new(64), Px::new(32)));
    }

    #[test]
    fn a_png_past_the_bound_or_of_a_size_it_cannot_give_is_refused() {
        let mut past = png_request(png(64, 32), (64, 32), JpegTurn::Stored);
        past.bound = Size::new(Px::new(63), Px::new(8192));
        assert_eq!(
            decode_png(&past),
            Err(RasterRefusal::PastBound {
                stored: Size::new(Px::new(64), Px::new(32)),
                bound: past.bound,
            })
        );
        assert_eq!(
            decode_png(&png_request(png(64, 32), (65, 32), JpegTurn::Stored)),
            Err(RasterRefusal::SizeUnavailable)
        );
        assert_eq!(
            decode_png(&png_request(b"GIF89a".to_vec(), (1, 1), JpegTurn::Stored)),
            Err(RasterRefusal::OtherFormat)
        );
        let mut cut = png(64, 32);
        cut.truncate(cut.len() - 20);
        assert_eq!(
            decode_png(&png_request(cut, (64, 32), JpegTurn::Stored)),
            Err(RasterRefusal::Undecodable)
        );
    }

    #[test]
    fn a_gif_decodes_into_its_frames_each_scaled_with_its_delay() {
        let frames = decode_gif(&gif_request(gif(3, 96, 32, 70), (48, 16), usize::MAX))
            .expect("the animation decodes");
        assert_eq!(frames.size, Size::new(Px::new(48), Px::new(16)));
        assert_eq!(frames.frames.len(), 3);
        assert!(frames.frames.iter().all(|f| f.delay == TimeSpan::new(70)));
        assert!(
            frames
                .frames
                .iter()
                .all(|f| f.rgba.as_bytes().len() == 48 * 16 * 4)
        );
        assert_ne!(frames.frames[1].rgba, frames.frames[2].rgba);
        let first = frames.into_first().expect("a first frame");
        assert_eq!(first.size, Size::new(Px::new(48), Px::new(16)));
    }

    #[test]
    fn the_frames_after_the_first_stop_at_the_budget() {
        let frame = 16 * 16 * 4;
        let alone = decode_gif(&gif_request(gif(4, 16, 16, 50), (16, 16), 0))
            .expect("the animation decodes");
        assert_eq!(alone.frames.len(), 1);
        let two = decode_gif(&gif_request(gif(4, 16, 16, 50), (16, 16), 2 * frame + 1))
            .expect("the animation decodes");
        assert_eq!(two.frames.len(), 2);
    }

    #[test]
    fn a_gif_past_the_bound_or_broken_is_refused_and_a_broken_tail_ends_it() {
        let mut past = gif_request(gif(1, 64, 32, 50), (64, 32), 0);
        past.bound = Size::new(Px::new(64), Px::new(31));
        assert!(matches!(
            decode_gif(&past),
            Err(RasterRefusal::PastBound { .. })
        ));
        assert_eq!(
            decode_gif(&gif_request(png(4, 4), (4, 4), 0)),
            Err(RasterRefusal::OtherFormat)
        );
        let whole = gif(3, 64, 32, 50);
        let cut = whole.get(..whole.len() - 30).expect("long").to_vec();
        // A broken tail keeps the frames before it.
        if let Ok(frames) = decode_gif(&gif_request(cut, (64, 32), usize::MAX)) {
            assert!(!frames.frames.is_empty() && frames.frames.len() < 3);
        }
        let head = whole.get(..20).expect("long").to_vec();
        assert_eq!(
            decode_gif(&gif_request(head, (64, 32), usize::MAX)),
            Err(RasterRefusal::Undecodable)
        );
    }

    #[test]
    fn a_frame_drawn_over_part_of_the_screen_keeps_what_lies_around_it() {
        // A full red frame, then a 2x2 blue one at (1, 1) whose disposal
        // clears it. The second shows red around blue. The third shows the
        // cleared square transparent.
        let mut canvas = Canvas::new(Size::new(Px::new(4), Px::new(4)));
        let red = gif::Frame {
            width: 4,
            height: 4,
            buffer: std::borrow::Cow::Owned([200, 0, 0, 255].repeat(16)),
            ..gif::Frame::default()
        };
        let blue = gif::Frame {
            left: 1,
            top: 1,
            width: 2,
            height: 2,
            dispose: gif::DisposalMethod::Background,
            buffer: std::borrow::Cow::Owned([0, 0, 200, 255].repeat(4)),
            ..gif::Frame::default()
        };
        let empty = gif::Frame {
            width: 1,
            height: 1,
            buffer: std::borrow::Cow::Owned(vec![0, 0, 0, 0]),
            ..gif::Frame::default()
        };
        canvas.compose(&red);
        let second = canvas.compose(&blue);
        assert_eq!(second.get(..4), Some([200, 0, 0, 255].as_slice()));
        assert_eq!(second.get(20..24), Some([0, 0, 200, 255].as_slice()));
        let third = canvas.compose(&empty);
        assert_eq!(third.get(20..24), Some([0, 0, 0, 0].as_slice()));
        assert_eq!(third.get(..4), Some([200, 0, 0, 255].as_slice()));
    }
}
