//! The image of a message, fetched and decoded for its preview
//! (`specs/5-Murmure/features/ambre-spec-murmure-attachments.md`).
//!
//! The view asks for an image it paints. This thread fetches it under the
//! channel's meta and decodes it here, off the render thread
//! (`ambre-spec-murmure-attachments.md`). The view gets frames at the size of the preview, never
//! the file, so a frame costs it a texture and no decode.

use std::io::Cursor;

use super::*;
use ambre_bus::{
    ChannelId, MediaRef, Millis, MurmureEvent, Picture, PictureFrame, RestCommand, RestEvent,
};
use ambre_core::{ByteSize, ImageSize};
use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};

/// The largest preview, in pixels: twice the box the list draws it in, so a
/// dense screen shows it sharp.
const PREVIEW_EDGE: ImageSize = ImageSize::new(480, 320);
/// The largest size a file may declare, each edge apart. A decode expands it
/// to four bytes a pixel before it is scaled down, so the header is held to
/// this first.
const MAX_SIZE: ImageSize = ImageSize::new(4096, 4096);
/// The most a decoder may allocate for one image.
const MAX_ALLOC: ByteSize = ByteSize(128 * 1024 * 1024);
/// The bytes the frames of one preview may hold. An animation past it plays
/// the frames that fit (`ambre-spec-budgets.md section 6`).
pub(crate) const PICTURE_BYTES: ByteSize = ByteSize(8 * 1024 * 1024);
/// A frame delay at or under this plays at [`SLOW_DELAY_MS`], as browsers do:
/// a file that asks for none would otherwise spin.
const FAST_DELAY_MS: Millis = Millis(10);
const SLOW_DELAY_MS: Millis = Millis(100);

impl LogicState {
    /// Fetch the image `media_ref`, stored under `document_id`, for the view
    /// of `channel_id`. One already on its way is not asked twice.
    pub(crate) fn show_picture(
        &mut self,
        channel_id: ChannelId,
        document_id: DocumentId,
        media_ref: MediaRef,
    ) {
        let asked = (channel_id, media_ref);
        if self.picturing.values().any(|held| *held == asked) {
            return;
        }
        let id = RequestId::next();
        let content_hash = ambre_core::ContentHash::from(asked.1.as_str());
        self.picturing.insert(id, asked);
        self.send_to_sync(SyncCommand::Rest {
            id,
            cmd: RestCommand::DownloadMedia {
                document_id,
                content_hash,
            },
        });
    }

    /// The answer to an image fetched for a preview: decoded, and handed to
    /// the view. Returns `false` for any other answer.
    pub(crate) fn pictured(&mut self, id: RequestId, evt: &RestEvent) -> bool {
        let Some((channel_id, media_ref)) = self.picturing.remove(&id) else {
            return false;
        };
        let picture = match evt {
            RestEvent::MediaDownloaded { data, .. } => decode(data.as_bytes()),
            _ => None,
        };
        self.send_picture(channel_id, media_ref, picture);
        true
    }

    /// An image the relay did not serve: the view keeps its chip. Returns
    /// `false` for any other request.
    pub(crate) fn picture_failed(&mut self, id: RequestId) -> bool {
        let Some((channel_id, media_ref)) = self.picturing.remove(&id) else {
            return false;
        };
        self.send_picture(channel_id, media_ref, None);
        true
    }

    fn send_picture(&self, channel_id: ChannelId, media_ref: MediaRef, picture: Option<Picture>) {
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::Picture { media_ref, picture },
        });
    }
}

/// Decode `bytes` into the frames of a preview. `None` for a type this
/// client does not preview, a header past [`MAX_SIZE`], or a broken file.
pub(crate) fn decode(bytes: &[u8]) -> Option<Picture> {
    let format = image::guess_format(bytes).ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIZE.width);
    limits.max_image_height = Some(MAX_SIZE.height);
    limits.max_alloc = Some(MAX_ALLOC.0);
    match format {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes)).ok()?;
            decoder.set_limits(limits).ok()?;
            let (width, height) = decoder.dimensions();
            let size = fit(ImageSize::new(width, height));
            let mut frames = Vec::new();
            let mut held = ByteSize(0);
            for frame in decoder.into_frames() {
                // A broken frame ends the animation; the frames before it play.
                let Ok(frame) = frame else { break };
                let (numer, denom) = frame.delay().numer_denom_ms();
                let pixels = scale(frame.into_buffer(), size);
                let after = held.saturating_add(ByteSize::of_len(pixels.len()));
                if !frames.is_empty() && after > PICTURE_BYTES {
                    break;
                }
                held = after;
                frames.push(PictureFrame {
                    pixels,
                    delay_ms: delay(Millis(u64::from(numer.checked_div(denom).unwrap_or(0)))),
                });
            }
            (!frames.is_empty()).then_some(Picture { size, frames })
        }
        ImageFormat::Png | ImageFormat::Jpeg => {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(limits);
            let image = reader.decode().ok()?.into_rgba8();
            let (width, height) = image.dimensions();
            let size = fit(ImageSize::new(width, height));
            Some(Picture {
                size,
                frames: vec![PictureFrame {
                    pixels: scale(image, size),
                    delay_ms: ambre_core::Millis(0),
                }],
            })
        }
        _ => None,
    }
}

/// The size `size` takes inside [`PREVIEW_EDGE`], its aspect kept. An image
/// already inside it keeps its size.
fn fit(size: ImageSize) -> ImageSize {
    let ImageSize { width, height, .. } = size;
    let ImageSize {
        width: max_w,
        height: max_h,
        ..
    } = PREVIEW_EDGE;
    let scale = (f64::from(max_w) / f64::from(width.max(1)))
        .min(f64::from(max_h) / f64::from(height.max(1)))
        .min(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let to = |edge: u32| ((f64::from(edge) * scale).round() as u32).max(1);
    ImageSize::new(to(width), to(height))
}

/// The pixels of `image` at `size`.
fn scale(image: RgbaImage, size: ImageSize) -> Vec<u8> {
    if image.dimensions() == (size.width, size.height) {
        return image.into_raw();
    }
    image::imageops::resize(&image, size.width, size.height, FilterType::Triangle).into_raw()
}

fn delay(ms: Millis) -> Millis {
    if ms <= FAST_DELAY_MS {
        SLOW_DELAY_MS
    } else {
        ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, Rgba};

    fn gif(frames: usize, (width, height): (u32, u32), delay_ms: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).expect("repeat");
            for i in 0..frames {
                #[allow(clippy::cast_possible_truncation)]
                let shade = (i * 40) as u8;
                let buffer = RgbaImage::from_pixel(width, height, Rgba([shade, 0, 0, 255]));
                let delay = Delay::from_numer_denom_ms(delay_ms, 1);
                encoder
                    .encode_frame(Frame::from_parts(buffer, 0, 0, delay))
                    .expect("frame");
            }
        }
        bytes
    }

    fn png((width, height): (u32, u32)) -> Vec<u8> {
        let mut bytes = Vec::new();
        RgbaImage::from_pixel(width, height, Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .expect("png");
        bytes
    }

    /// An animated GIF decodes here into its frames, each
    /// with its delay, scaled into the preview. A delay of none plays slow.
    #[test]
    fn an_animated_gif_decodes_into_its_frames_at_the_preview_size() {
        let picture = decode(&gif(3, (960, 320), 70)).expect("a GIF");
        assert_eq!(picture.size, ImageSize::new(480, 160));
        assert_eq!(picture.frames.len(), 3);
        assert!(
            picture
                .frames
                .iter()
                .all(|f| f.delay_ms == ambre_core::Millis(70))
        );
        assert!(
            picture
                .frames
                .iter()
                .all(|f| f.pixels.len() == 480 * 160 * 4)
        );
        assert_ne!(picture.frames[0].pixels, picture.frames[1].pixels);
        let spinning = decode(&gif(2, (8, 8), 0)).expect("a GIF");
        assert!(spinning.frames.iter().all(|f| f.delay_ms == SLOW_DELAY_MS));
    }

    /// A still image is one frame, and a small one keeps its size. A file
    /// this client does not preview, or a broken one, decodes to nothing.
    #[test]
    fn a_still_image_is_one_frame_and_a_stranger_none() {
        let picture = decode(&png((40, 30))).expect("a PNG");
        assert_eq!(picture.size, ImageSize::new(40, 30));
        assert_eq!(picture.frames.len(), 1);
        assert!(decode(b"%PDF-1.7").is_none());
        assert!(decode(&png((40, 30))[..20]).is_none());
    }

    /// A header past the bound is refused before its pixels are allocated.
    #[test]
    fn a_header_past_the_bound_decodes_nothing() {
        let mut big = png((1, 1));
        // The IHDR width, past `MAX_SIZE`; the CRC no longer matches either.
        big[16..20].copy_from_slice(&(MAX_SIZE.width + 1).to_be_bytes());
        assert!(decode(&big).is_none());
    }

    /// An animation past the byte bound keeps the frames that fit.
    #[test]
    fn an_animation_past_the_bound_keeps_the_frames_that_fit() {
        let frame = 480 * 320 * 4;
        let fit = PICTURE_BYTES.as_len() / frame;
        let picture = decode(&gif(fit + 3, (480, 320), 50)).expect("a GIF");
        assert_eq!(picture.frames.len(), fit);
        assert!(picture.bytes() <= PICTURE_BYTES);
    }
}
