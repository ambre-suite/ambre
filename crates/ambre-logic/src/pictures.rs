//! The image of a message, fetched and decoded for its preview
//! (`specs/5-Murmure/features/ambre-spec-murmure-attachments.md`).
//!
//! The view asks for an image it paints. This thread fetches it under the
//! channel's meta and reads its header. The converter thread decodes it, off
//! the render thread and off this one (`ARCH-097`). A PNG, a
//! JPEG or a GIF decodes there, in the images module. The bytes move into the
//! job with no copy. The view gets frames at the size of the preview, never
//! the file, so a frame costs it a texture and no decode.

use ambre_core::Validated;
use ambre_core::{Image, Px, Size};

use super::*;
use ambre_bus::{ContentHash, Ms, RestCommand, RestEvent, TimeSpan};
use ambre_core::{Count, MediaBytes, Octets, Payload};

/// The largest preview, in pixels: twice the box the list draws it in, so a
/// dense screen shows it sharp.
const PREVIEW_EDGE: Size<Px<Image>> = Size::new(Px::new(480), Px::new(320));
/// The largest size a file may declare, each edge apart. A decode expands it
/// to four bytes a pixel before it is scaled down, so the header is held to
/// this first.
const MAX_SIZE: Size<Px<Image>> = Size::new(Px::new(4096), Px::new(4096));
/// The bytes the frames of one preview may hold. An animation past it plays
/// the frames that fit (`ambre-spec-budgets.md section 6`).
pub(crate) const PICTURE_MAX_SIZE: Count<Octets> = Count::<Octets>::new(8 * 1024 * 1024);
/// A frame delay at or under this plays at [`SLOW_DELAY`], as browsers do:
/// a file that asks for none would otherwise spin.
const FAST_DELAY: TimeSpan<Ms> = TimeSpan::<Ms>::new(10);
const SLOW_DELAY: TimeSpan<Ms> = TimeSpan::<Ms>::new(100);

/// A decoded preview: the frames at the size the view paints. The dispatch
/// hands it to the view as Murmure's picture (`ARCH-121`).
#[derive(Debug)]
pub(crate) struct Preview {
    /// The size of every frame.
    pub(crate) size: Size<Px<Image>>,
    /// Each frame in play order, never empty.
    pub(crate) frames: Vec<PreviewFrame>,
}

/// One frame of a [`Preview`]: non-premultiplied RGBA8, row-major, shown for
/// `delay`.
#[derive(Debug)]
pub(crate) struct PreviewFrame {
    pub(crate) pixels: Payload<ambre_core::RgbaPixels>,
    pub(crate) delay: TimeSpan<Ms>,
}

impl Preview {
    /// The bytes its frames hold.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn bytes(&self) -> Count<Octets> {
        self.frames.iter().map(|f| f.pixels.len()).sum()
    }
}

impl LogicState {
    /// Fetch the image `media_ref`, stored under `document_id`, for the view
    /// of `channel_id`. One already on its way is not asked twice.
    pub(crate) fn show_picture(
        &mut self,
        channel_id: Id<Channel>,
        document_id: Id<Document>,
        media_ref: Validated<ContentHash>,
    ) {
        let asked = (channel_id, media_ref);
        if self.picturing.values().any(|held| *held == asked) {
            return;
        }
        let id = ambre_bus::next_request();
        let content_hash = asked.1.clone();
        self.picturing.insert(id, asked);
        self.send_to_sync(SyncCommand::Rest {
            id,
            cmd: RestCommand::DownloadMedia {
                document_id,
                content_hash,
            },
        });
    }

    /// The answer to an image fetched for a preview: its decode leaves for
    /// the converter thread (`ARCH-097`). The answer reaches the view on a
    /// later pass, through `previewed`. Gives `evt` back when it answers no
    /// preview request.
    pub(crate) fn pictured(&mut self, id: Seq<Request>, evt: RestEvent) -> Option<RestEvent> {
        let Some((channel_id, media_ref)) = self.picturing.remove(&id) else {
            return Some(evt);
        };
        // The bytes move into the job: the Logic thread copies none.
        let conversion = match evt {
            RestEvent::MediaDownloaded { data, .. } => preview_decode(data),
            _ => None,
        };
        match conversion {
            Some(conversion) => self.offload(crate::converter_thread::Job {
                ticket: id,
                owner: crate::converter_thread::JobOwner::Murmure {
                    channel_id,
                    media_ref,
                },
                conversion,
            }),
            None => self.send_picture(channel_id, media_ref, None),
        }
        None
    }

    /// The answer of the converter thread for the preview of `media_ref`. A
    /// module that did not load shows no preview, and so does a file it
    /// refused. The view keeps its chip, the file stays downloadable, and the
    /// next paint asks again (`ARCH-081`).
    pub(crate) fn previewed(
        &mut self,
        channel_id: Id<Channel>,
        media_ref: Validated<ContentHash>,
        answer: Result<ambre_convert::Converted, crate::converter_thread::JobFailure>,
    ) {
        let picture = answer.ok().and_then(picture_of);
        self.send_picture(channel_id, media_ref, picture);
    }

    /// An image the relay did not serve: the view keeps its chip. Returns
    /// `false` for any other request.
    pub(crate) fn picture_failed(&mut self, id: Seq<Request>) -> bool {
        let Some((channel_id, media_ref)) = self.picturing.remove(&id) else {
            return false;
        };
        self.send_picture(channel_id, media_ref, None);
        true
    }
}

/// The decode the images module runs for the preview of `media`: the header
/// held to [`MAX_SIZE`] here, the samples scaled into the preview there.
/// `None` for a file this client does not preview, or a header past the
/// bound or broken: it shows no preview, and no job leaves.
fn preview_decode(media: Payload<MediaBytes>) -> Option<ambre_convert::Conversion> {
    use ambre_convert::{Conversion, jpeg, raster};
    let within = |size: Size<Px<Image>>| {
        (size.width <= MAX_SIZE.width && size.height <= MAX_SIZE.height).then(|| fit(size))
    };
    if let Some(size) = jpeg::stored_size(&media) {
        return Some(Conversion::DecodeJpeg(jpeg::JpegDecode {
            size: within(size)?,
            data: media.into_role(),
            bound: MAX_SIZE,
            turn: jpeg::JpegTurn::Stored,
        }));
    }
    if let Some(size) = raster::png_size(&media) {
        return Some(Conversion::DecodePng(raster::PngDecode {
            size: within(size)?,
            data: media.into_role(),
            bound: MAX_SIZE,
            turn: jpeg::JpegTurn::Stored,
        }));
    }
    let size = within(raster::gif_size(&media)?)?;
    Some(Conversion::DecodeGif(raster::GifDecode {
        data: media.into_role(),
        bound: MAX_SIZE,
        size,
        budget: PICTURE_MAX_SIZE,
    }))
}

/// The preview the images module answered: one frame for a still picture,
/// each frame of an animation with its delay. A delay of none plays slow.
/// `None` for an answer of another kind.
fn picture_of(converted: ambre_convert::Converted) -> Option<Preview> {
    match converted {
        ambre_convert::Converted::Pixels(pixels) => Some(Preview {
            size: pixels.size,
            frames: vec![PreviewFrame {
                pixels: pixels.rgba,
                delay: TimeSpan::<Ms>::new(0),
            }],
        }),
        ambre_convert::Converted::Frames(animation) => {
            let frames: Vec<PreviewFrame> = animation
                .frames
                .into_iter()
                .map(|frame| PreviewFrame {
                    pixels: frame.rgba,
                    delay: delay(frame.delay),
                })
                .collect();
            (!frames.is_empty()).then_some(Preview {
                size: animation.size,
                frames,
            })
        }
        _ => None,
    }
}

/// The size `size` takes inside [`PREVIEW_EDGE`], its aspect kept. An image
/// already inside it keeps its size.
fn fit(size: Size<Px<Image>>) -> Size<Px<Image>> {
    let (width, height) = (size.width.get(), size.height.get());
    let (max_w, max_h) = (PREVIEW_EDGE.width.get(), PREVIEW_EDGE.height.get());
    let scale = (f64::from(max_w) / f64::from(width.max(1)))
        .min(f64::from(max_h) / f64::from(height.max(1)))
        .min(1.0);
    let to = |edge: u32| Px::new(((f64::from(edge) * scale).round() as u32).max(1));
    Size::new(to(width), to(height))
}

fn delay(ms: TimeSpan<Ms>) -> TimeSpan<Ms> {
    if ms <= FAST_DELAY { SLOW_DELAY } else { ms }
}

// The decodes run through the images engine, which the host alone links.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use ambre_core::{Ms, TimeSpan};
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    fn gif(frames: usize, (width, height): (u32, u32), delay_ms: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).expect("repeat");
            for i in 0..frames {
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

    /// The preview of `file`, its decode run through the images engine the
    /// host links, as the converter thread runs it. `None` where no job
    /// leaves or the module refuses the file.
    fn preview(file: Vec<u8>) -> Option<Preview> {
        let conversion = preview_decode(Payload::new(file))?;
        let converter = ambre_convert::Converter::load(
            ambre_core::ConverterFamily::Images,
            &Payload::new(Vec::new()),
        )
        .expect("the host links the images engine");
        converter.convert(&conversion).ok().and_then(picture_of)
    }

    /// An animated GIF decodes on the converter thread into its frames, each
    /// with its delay, scaled into the preview. A delay of none plays slow.
    #[test]
    fn an_animated_gif_decodes_into_its_frames_at_the_preview_size() {
        let picture = preview(gif(3, (960, 320), 70)).expect("a GIF");
        assert_eq!(picture.size, Size::new(Px::new(480), Px::new(160)));
        assert_eq!(picture.frames.len(), 3);
        assert!(
            picture
                .frames
                .iter()
                .all(|f| f.delay == TimeSpan::<Ms>::new(70))
        );
        assert!(
            picture
                .frames
                .iter()
                .all(|f| f.pixels.len()
                    == ambre_core::Count::<ambre_core::Octets>::new(480 * 160 * 4))
        );
        assert_ne!(picture.frames[0].pixels, picture.frames[1].pixels);
        let spinning = preview(gif(2, (8, 8), 0)).expect("a GIF");
        assert!(spinning.frames.iter().all(|f| f.delay == SLOW_DELAY));
    }

    /// A still image is one frame, and a small one keeps its size. A file
    /// this client does not preview leaves no job, and a broken one decodes
    /// to nothing.
    #[test]
    fn a_still_image_is_one_frame_and_a_stranger_none() {
        let picture = preview(png((40, 30))).expect("a PNG");
        assert_eq!(picture.size, Size::new(Px::new(40), Px::new(30)));
        assert_eq!(picture.frames.len(), 1);
        assert!(preview_decode(Payload::new(b"%PDF-1.7".to_vec())).is_none());
        assert!(preview(png((40, 30))[..40].to_vec()).is_none());
    }

    /// The bytes of a JPEG, a PNG or a GIF preview move into its job. The
    /// Logic thread copies none of them on the way to the converter thread.
    #[test]
    fn the_bytes_of_a_preview_move_into_its_job_uncopied() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image::RgbImage::from_pixel(8, 8, image::Rgb([200, 0, 0])))
            .expect("a JPEG encodes");
        for file in [jpeg, png((40, 30)), gif(2, (8, 8), 50)] {
            let media: Payload<MediaBytes> = Payload::new(file);
            let at = media.as_bytes().as_ptr();
            let data = match preview_decode(media).expect("a job leaves") {
                ambre_convert::Conversion::DecodeJpeg(asked) => asked.data,
                ambre_convert::Conversion::DecodePng(asked) => asked.data,
                ambre_convert::Conversion::DecodeGif(asked) => asked.data,
                other => panic!("a picture decode, got {other:?}"),
            };
            assert_eq!(
                data.as_bytes().as_ptr(),
                at,
                "the buffer is the one that arrived"
            );
        }
    }

    /// A header past the bound leaves no job, so no pixel is allocated.
    #[test]
    fn a_header_past_the_bound_decodes_nothing() {
        let mut big = png((1, 1));
        // The IHDR width, past `MAX_SIZE`; the CRC no longer matches either.
        big[16..20].copy_from_slice(&(MAX_SIZE.width.get() + 1).to_be_bytes());
        assert!(preview_decode(Payload::new(big)).is_none());
    }

    /// An animation past the byte bound keeps the frames that fit.
    #[test]
    fn an_animation_past_the_bound_keeps_the_frames_that_fit() {
        let frame = 480 * 320 * 4;
        let fit = PICTURE_MAX_SIZE.saturating_len().to_usize() / frame;
        let picture = preview(gif(fit + 3, (480, 320), 50)).expect("a GIF");
        assert_eq!(picture.frames.len(), fit);
        assert!(picture.bytes() <= PICTURE_MAX_SIZE);
    }
}
