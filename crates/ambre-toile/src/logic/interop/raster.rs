//! Raster image import (`toile-import-export`).
//!
//! Decodes PNG/JPG bytes with the pure-Rust `image` crate, bounds the decoded
//! dimensions against [`Limits`], and builds a small placeholder thumbnail for
//! the first paint. It then emits a single `image` `CreateElement` plus the
//! plaintext [`MediaBlob`] the Sync layer will encrypt/upload. Interop never
//! hashes: the blob's content address is a **pre-upload placeholder token**
//! (`pending:{n}`) that Phase 1 rewrites to the real blake2b once Sync returns
//! `MediaUploaded { content_hash }`.

use std::io::Cursor;

use image::{GenericImageView, ImageFormat};
use kurbo::{Point, Size};

use super::{ContentHash, ImportBatch, ImportError, Limits, MediaBlob};
use crate::logic::command::{InitContent, ToileCommand};
use crate::logic::model::ElementKind;

/// Largest edge (px) of the placeholder thumbnail stored inline in the media
/// entry. Small enough to paint instantly, large enough to preview.
const THUMB_MAX_EDGE: u32 = 256;

/// The placeholder content-address token minted for the `n`-th media blob of an
/// import. Not a hash -- interop does not hash. Phase 1 replaces it with the
/// blake2b `content_hash` the Sync thread returns.
pub(super) fn pending_hash(index: usize) -> ContentHash {
    ContentHash(format!("pending:{index}"))
}

/// Import a single raster file: decode, bound, thumbnail, and place one
/// `image` element at the origin sized to the native (clamped) resolution. The
/// full-res plaintext rides in the returned [`MediaBlob`]; the element references
/// it by the placeholder token.
///
/// # Errors
///
/// [`ImportError::Decode`] when the decoder refuses `bytes`, and
/// [`ImportError::ImageTooLarge`] when the decoded image exceeds `limits`.
pub fn import_raster(bytes: &[u8], limits: &Limits) -> Result<ImportBatch, ImportError> {
    let mut batch = ImportBatch::default();
    let hash = pending_hash(0);
    let blob = decode_media(bytes, limits, hash.clone())?;
    let size = Size::new(f64::from(blob.width), f64::from(blob.height));
    batch.commands.push(ToileCommand::CreateElement {
        kind: ElementKind::Image,
        at: Point::ZERO,
        size,
        content: InitContent::Image(hash.0.clone()),
        fill: None,
        correlate: 0,
    });
    batch.media.push(blob);
    batch.report.imported = 1;
    Ok(batch)
}

/// Decode raster bytes into a [`MediaBlob`] under `hash`, enforcing the image
/// dimension bound. Shared by [`import_raster`] and the SVG `<image>` path.
pub(super) fn decode_media(
    bytes: &[u8],
    limits: &Limits,
    hash: ContentHash,
) -> Result<MediaBlob, ImportError> {
    let format = image::guess_format(bytes).map_err(|e| ImportError::Decode(e.to_string()))?;
    // The header first (`TOILE-IO-052`). A decoded picture is four bytes per
    // pixel, so measuring one after expanding it bounds the document and not
    // the heap. A file declaring 40 000 squared costs six gigabytes before the
    // bound is read. Reading the dimensions off the header costs the header.
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImportError::Decode(e.to_string()))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| ImportError::Decode(e.to_string()))?;
    limits.check_image_dims(width, height)?;

    // A header that lies about its size is the second way in, so the decoder
    // carries the same bound and refuses to allocate past it.
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ImportError::Decode(e.to_string()))?;
    let mut alloc = image::Limits::default();
    alloc.max_image_width = Some(limits.max_image_dim);
    alloc.max_image_height = Some(limits.max_image_dim);
    reader.limits(alloc);
    let decoded = reader
        .decode()
        .map_err(|e| ImportError::Decode(e.to_string()))?;
    let (width, height) = decoded.dimensions();
    limits.check_image_dims(width, height)?;

    // Downscale to a placeholder thumbnail (preserves aspect ratio). PNG-encoded
    // so it round-trips losslessly and the UI can paint it before the full-res
    // blob arrives.
    let thumb_img = decoded.thumbnail(THUMB_MAX_EDGE, THUMB_MAX_EDGE);
    let mut thumb = Vec::new();
    thumb_img
        .write_to(&mut Cursor::new(&mut thumb), ImageFormat::Png)
        .map_err(|e| ImportError::Decode(e.to_string()))?;

    Ok(MediaBlob {
        hash,
        plaintext: bytes.to_vec(),
        mime: mime_of(format).to_owned(),
        width,
        height,
        thumb,
    })
}

/// Map a decoded [`ImageFormat`] to its MIME string for the media entry.
fn mime_of(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        // Only png/jpeg are compiled in (Cargo feature set); anything else that
        // slips through guess_format is labeled generically.
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal in-memory PNG of the given solid size, encoded with `image` so
    /// the test never reads a fixture file for the decode path.
    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn imports_one_image_with_thumbnail() {
        let png = solid_png(64, 48);
        let batch = import_raster(&png, &Limits::default()).unwrap();
        assert_eq!(batch.commands.len(), 1);
        assert_eq!(batch.media.len(), 1);
        assert_eq!(batch.report.imported, 1);

        let blob = &batch.media[0];
        assert_eq!((blob.width, blob.height), (64, 48));
        assert_eq!(blob.mime, "image/png");
        assert!(
            !blob.thumb.is_empty(),
            "a placeholder thumbnail is produced"
        );
        assert_eq!(blob.plaintext, png, "full-res plaintext is preserved");

        match &batch.commands[0] {
            ToileCommand::CreateElement {
                kind: ElementKind::Image,
                size,
                content: InitContent::Image(h),
                ..
            } => {
                assert_eq!(*size, Size::new(64.0, 48.0));
                assert_eq!(h, &blob.hash.0, "element references its blob token");
            }
            other => panic!("expected an image CreateElement, got {other:?}"),
        }
    }

    #[test]
    fn oversized_image_aborts_atomically() {
        let png = solid_png(300, 20);
        let limits = Limits {
            max_image_dim: 256,
            ..Limits::default()
        };
        let err = import_raster(&png, &limits).unwrap_err();
        assert!(matches!(err, ImportError::ImageTooLarge { limit: 256, .. }));
    }

    /// `TOILE-IO-052`: the picture is measured before it is expanded. The
    /// bytes here carry a whole header and a truncated body, so a decoder that
    /// expanded first would answer with a decode failure. Measured first, the
    /// size is read off the header and the refusal names the bound.
    #[test]
    fn an_oversized_picture_is_refused_off_its_header() {
        let png = solid_png(300, 20);
        // Everything up to the first pixel chunk: the header is whole, the
        // image data is not there at all.
        let idat = png
            .windows(4)
            .position(|w| w == b"IDAT")
            .expect("a PNG data chunk");
        let header_only = png[..idat + 4].to_vec();
        let limits = Limits {
            max_image_dim: 256,
            ..Limits::default()
        };
        assert!(
            matches!(
                import_raster(&header_only, &limits).unwrap_err(),
                ImportError::ImageTooLarge {
                    width: 300,
                    height: 20,
                    limit: 256
                }
            ),
            "the picture was expanded before it was measured"
        );
    }

    #[test]
    fn thumbnail_is_bounded() {
        let png = solid_png(1024, 512);
        let blob = decode_media(&png, &Limits::default(), pending_hash(0)).unwrap();
        // The thumbnail decodes back to something within the max edge.
        let thumb = image::load_from_memory(&blob.thumb).unwrap();
        assert!(thumb.width() <= THUMB_MAX_EDGE && thumb.height() <= THUMB_MAX_EDGE);
    }

    #[test]
    fn garbage_bytes_fail_to_decode() {
        let err = import_raster(&[0u8, 1, 2, 3], &Limits::default()).unwrap_err();
        assert!(matches!(err, ImportError::Decode(_)));
    }
}
