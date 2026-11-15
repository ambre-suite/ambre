//! Criterion benchmark for the picture decodes of the images module: JPEG,
//! PNG, GIF and WebP, each through `ambre_convert::Converter` as the
//! converter thread calls it.
//!
//! The figures document the rows of `ambre-spec-budgets.md section 1` that
//! the converter thread holds: "Prose picture decode, one picture" and the
//! answer deadline of the converter thread. A photograph of 4032 by 3024
//! pixels decodes at the cache edge of Prose, upright, as a Prose page asks.
//! A GIF of ten frames decodes at the preview size, as Murmure asks. The
//! fixtures are written by the encoders of the `image` crate at run time,
//! from a seed of arithmetic, never from a real picture. No CI job runs it.

#![expect(clippy::panic, reason = "a bench aborts on a picture it cannot run")]

use std::hint::black_box;
use std::io::Cursor;

use ambre_convert::jpeg::{JpegDecode, JpegTurn};
use ambre_convert::raster::{GifDecode, PngDecode};
use ambre_convert::{Conversion, Converter};
use ambre_core::{ConverterFamily, Count, Image, Payload, Px, Size};
use criterion::{Criterion, criterion_group, criterion_main};

/// The photograph of a phone camera: 4032 by 3024 pixels.
const PHOTO: (u32, u32) = (4032, 3024);
/// The cache edge of Prose, and the size a photograph decodes to there.
const CACHE: (u32, u32) = (2048, 1536);
/// A GIF of a chat: ten frames of 640 by 480, shown at the preview size.
const GIF: (u32, u32) = (640, 480);
const PREVIEW: (u32, u32) = (427, 320);

fn size((width, height): (u32, u32)) -> Size<Px<Image>> {
    Size::new(Px::new(width), Px::new(height))
}

/// A picture with detail in every pixel, so no decoder takes a short path
/// over flat runs.
fn photo() -> image::RgbImage {
    image::RgbImage::from_fn(PHOTO.0, PHOTO.1, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
    })
}

fn jpeg() -> Vec<u8> {
    let mut out = Vec::new();
    if let Err(e) = image::codecs::jpeg::JpegEncoder::new(&mut out).encode_image(&photo()) {
        panic!("a JPEG encodes: {e}");
    }
    out
}

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    if let Err(e) = photo().write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png) {
        panic!("a PNG encodes: {e}");
    }
    out
}

fn webp() -> Vec<u8> {
    let mut out = Vec::new();
    let encoded = image_webp::WebPEncoder::new(&mut out).encode(
        photo().as_raw(),
        PHOTO.0,
        PHOTO.1,
        image_webp::ColorType::Rgb8,
    );
    if let Err(e) = encoded {
        panic!("a WebP encodes: {e}");
    }
    out
}

fn gif() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(&mut out, 30);
        for step in 0..10u32 {
            let frame = image::RgbaImage::from_fn(GIF.0, GIF.1, |x, y| {
                image::Rgba([((x + step * 8) % 256) as u8, (y % 256) as u8, 90, 255])
            });
            if let Err(e) = encoder.encode_frame(image::Frame::new(frame)) {
                panic!("a GIF frame encodes: {e}");
            }
        }
    }
    out
}

fn decodes(c: &mut Criterion) {
    let converter = Converter::load(ConverterFamily::Images, &Payload::new(Vec::new()))
        .unwrap_or_else(|e| panic!("the host links the images engine: {e:?}"));
    let bound = size((8192, 8192));
    let cases = [
        (
            "jpeg_photo_to_cache_edge",
            Conversion::DecodeJpeg(JpegDecode {
                data: Payload::new(jpeg()),
                bound,
                size: size(CACHE),
                turn: JpegTurn::Upright,
            }),
        ),
        (
            "png_photo_to_cache_edge",
            Conversion::DecodePng(PngDecode {
                data: Payload::new(png()),
                bound,
                size: size(CACHE),
                turn: JpegTurn::Upright,
            }),
        ),
        (
            "gif_ten_frames_to_preview",
            Conversion::DecodeGif(GifDecode {
                data: Payload::new(gif()),
                bound,
                size: size(PREVIEW),
                budget: Count::new(u64::MAX),
            }),
        ),
        (
            "webp_photo_whole",
            Conversion::DecodeWebp {
                data: Payload::new(webp()),
            },
        ),
    ];
    let mut group = c.benchmark_group("decode");
    group.sample_size(10);
    for (name, conversion) in cases {
        group.bench_function(name, |b| {
            b.iter(|| match converter.convert(black_box(&conversion)) {
                Ok(answer) => black_box(answer),
                Err(e) => panic!("{name} decodes: {e:?}"),
            });
        });
    }
    group.finish();
}

criterion_group!(benches, decodes);
criterion_main!(benches);
