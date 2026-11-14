//! A picture's pure rules: its bounds, its size, its decode and the cache of
//! decoded pictures (`features/ambre-spec-prose-images.md section 3`).
//!
//! Nothing here touches the document, the network or a key. The Logic
//! thread holds one [`PictureCache`] per document, and asks the Sync worker
//! for the bytes it decodes here.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use ambre_core::{
    ContentHash, Count, Doc, Image, MediaBytes, Octets, Payload, Px, RgbaPixels, Seq, Size,
    Validated,
};
use image::{ImageReader, Limits};

use crate::message::{
    AspectLock, DecodedPicture, DisplaySize, MediaFailure, Orientation, PictureShown,
    PrintedPicture,
};

/// The largest edge a source file may declare, each edge apart
/// (`PROSE-FEAT-314`). A decode expands the file before it scales it down,
/// so the header is held to this first (`ambre-spec-budgets.md section 6`).
pub const MAX_SOURCE_EDGE: Px<Image> = Px::new(8192);

/// The largest edge of a decoded picture the cache holds (`PROSE-FEAT-315`).
/// A full-width picture of a printed page is about 300 dots an inch at it.
pub const CACHE_EDGE: Px<Image> = Px::new(2048);

/// The largest display edge the document stores (`PROSE-FEAT-059`). Past it
/// no page can show the picture, and the layout cost is unbounded
/// (`ambre-spec-budgets.md section 6`).
pub const MAX_DISPLAY_EDGE: Px<Doc> = Px::new(4096.0);

/// The decoded bytes the cache of one document holds before it evicts
/// (`PROSE-FEAT-054`, `ambre-spec-budgets.md section 6`).
pub const CACHE_CAP: Count<Octets> = Count::new(64 * 1024 * 1024);

/// The most the decoder may allocate for one source picture: an RGB picture
/// at the source bound, before it is scaled down.
const MAX_DECODE_ALLOC: Count<Octets> = Count::new(256 * 1024 * 1024);

/// What a decode of an arrived picture asks for.
#[derive(Debug)]
pub enum Decode {
    /// A PNG, decoded here at the cache edge.
    Done(DecodedPicture),
    /// A JPEG: the decode the images module runs on the converter thread,
    /// its header already held to the source bound here. The Logic thread
    /// never decodes a JPEG, and never waits for one (`PROSE-FEAT-330`).
    Jpeg(ambre_convert::jpeg::JpegDecode),
}

/// Decode `bytes` into a picture of at most [`CACHE_EDGE`] a side, or say
/// which decode the images module runs for it.
///
/// The header is read first and held to [`MAX_SOURCE_EDGE`], so a file that
/// declares a huge size costs its header alone. The decoder carries the same
/// bound, so a header that lies is refused too.
///
/// # Errors
///
/// [`MediaFailure::Undecodable`] for a format this build does not decode, a
/// header past the bound, or a broken file.
///
/// The bytes are taken whole: a JPEG moves on to its request with no copy.
pub fn decode(bytes: Payload<MediaBytes>) -> Result<Decode, MediaFailure> {
    native_size(&bytes)?;
    let raw = stored_size(&bytes).ok_or(MediaFailure::Undecodable)?;
    if ambre_convert::jpeg::is_jpeg(&bytes) {
        return Ok(Decode::Jpeg(ambre_convert::jpeg::JpegDecode {
            data: bytes.into_role(),
            bound: Size::new(MAX_SOURCE_EDGE, MAX_SOURCE_EDGE),
            size: cache_size(raw),
            turn: ambre_convert::jpeg::JpegTurn::Upright,
        }));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE.get());
    limits.max_image_height = Some(MAX_SOURCE_EDGE.get());
    limits.max_alloc = Some(MAX_DECODE_ALLOC.get());
    let mut reader = ImageReader::new(Cursor::new(bytes.as_bytes()))
        .with_guessed_format()
        .map_err(|_| MediaFailure::Undecodable)?;
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| MediaFailure::Undecodable)?;
    let turn = image::ImageDecoder::orientation(&mut decoder)
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let decoded =
        image::DynamicImage::from_decoder(decoder).map_err(|_| MediaFailure::Undecodable)?;
    let size = cache_size(raw);
    let (width, height) = (size.width.get(), size.height.get());
    let filter = image::imageops::FilterType::Triangle;
    // Main links the scale for RGB and RGBA samples alone, which every other
    // kind is turned into first.
    let rgba = match decoded {
        image::DynamicImage::ImageRgb8(rgb) if size != raw => {
            image::DynamicImage::ImageRgb8(image::imageops::resize(&rgb, width, height, filter))
                .into_rgba8()
        }
        other if size != raw => image::imageops::resize(&other.into_rgba8(), width, height, filter),
        other => other.into_rgba8(),
    };
    let turned = turn_upright(rgba, turn);
    let (width, height) = turned.dimensions();
    Ok(Decode::Done(DecodedPicture {
        pixels: Payload::<RgbaPixels>::new(turned.into_raw()),
        size: Size::new(Px::new(width), Px::new(height)),
    }))
}

/// The picture the images module decoded for a [`Decode::Jpeg`].
#[must_use]
pub fn from_pixels(pixels: ambre_convert::images::Pixels) -> DecodedPicture {
    DecodedPicture {
        pixels: pixels.rgba,
        size: pixels.size,
    }
}

/// [`decode`] carried to its end in one call: a JPEG through the images
/// engine the host links, as the converter thread runs it. Tests alone.
#[cfg(test)]
pub(crate) fn decode_here(bytes: &Payload<MediaBytes>) -> Result<DecodedPicture, MediaFailure> {
    match decode(bytes.clone())? {
        Decode::Done(decoded) => Ok(decoded),
        Decode::Jpeg(request) => {
            let converter = ambre_convert::Converter::load(
                ambre_core::ConverterFamily::Images,
                &Payload::new(Vec::new()),
            )
            .map_err(|_| MediaFailure::Unavailable)?;
            match converter.convert(&ambre_convert::Conversion::DecodeJpeg(request)) {
                Ok(ambre_convert::Converted::Pixels(pixels)) => Ok(from_pixels(pixels)),
                _ => Err(MediaFailure::Undecodable),
            }
        }
    }
}

/// `picture` turned as `turn` states, on RGBA samples alone.
fn turn_upright(picture: image::RgbaImage, turn: image::metadata::Orientation) -> image::RgbaImage {
    use image::imageops::{flip_horizontal, flip_vertical, rotate90, rotate180, rotate270};
    use image::metadata::Orientation as Turn;
    match turn {
        Turn::NoTransforms => picture,
        Turn::Rotate90 => rotate90(&picture),
        Turn::Rotate180 => rotate180(&picture),
        Turn::Rotate270 => rotate270(&picture),
        Turn::FlipHorizontal => flip_horizontal(&picture),
        Turn::FlipVertical => flip_vertical(&picture),
        Turn::Rotate90FlipH => flip_horizontal(&rotate90(&picture)),
        Turn::Rotate270FlipH => flip_horizontal(&rotate270(&picture)),
    }
}

/// The format a file of a package names for `bytes`, read off their
/// signature. `None` for a format no office file holds.
#[must_use]
pub fn file_format(bytes: &Payload<MediaBytes>) -> Option<ambre_prose_form::ImageFormat> {
    use ambre_prose_form::ImageFormat as Format;
    Some(match image::guess_format(bytes.as_bytes()).ok()? {
        image::ImageFormat::Png => Format::Png,
        image::ImageFormat::Jpeg => Format::Jpeg,
        image::ImageFormat::Gif => Format::Gif,
        image::ImageFormat::Bmp => Format::Bmp,
        image::ImageFormat::Tiff => Format::Tiff,
        _ => return None,
    })
}

/// The size a source file declares, held to [`MAX_SOURCE_EDGE`].
///
/// # Errors
///
/// [`MediaFailure::Undecodable`] for a header this build cannot read, or one
/// past the bound.
pub fn native_size(bytes: &Payload<MediaBytes>) -> Result<Size<Px<Image>>, MediaFailure> {
    let raw = stored_size(bytes).ok_or(MediaFailure::Undecodable)?;
    let zero = Px::<Image>::new(0);
    if raw.width == zero
        || raw.height == zero
        || raw.width > MAX_SOURCE_EDGE
        || raw.height > MAX_SOURCE_EDGE
    {
        return Err(MediaFailure::Undecodable);
    }
    // The size the picture shows at, its orientation applied.
    Ok(if orientation_of(bytes).swaps_sides() {
        Size::new(raw.height, raw.width)
    } else {
        raw
    })
}

/// The size a file's header declares, its orientation not applied. A JPEG
/// header is walked here: Main links no JPEG decoder (`ARCH-078`).
fn stored_size(bytes: &Payload<MediaBytes>) -> Option<Size<Px<Image>>> {
    if ambre_convert::jpeg::is_jpeg(bytes) {
        return ambre_convert::jpeg::stored_size(bytes);
    }
    let (width, height) = ImageReader::new(Cursor::new(bytes.as_bytes()))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    Some(Size::new(Px::new(width), Px::new(height)))
}

/// The orientation a JPEG's EXIF states, or upright for any other file.
#[must_use]
pub fn orientation_of(bytes: &Payload<MediaBytes>) -> Orientation {
    orientation_in(bytes.as_bytes())
}

/// The kernel of [`orientation_of`], over the bytes a strip walks too.
fn orientation_in(bytes: &[u8]) -> Orientation {
    let mut at = JPEG_SOI.len();
    if !bytes.starts_with(&JPEG_SOI) {
        return Orientation::Upright;
    }
    while let Some((marker, payload, next)) = jpeg_segment(bytes, at) {
        if marker == APP1
            && let Some(tiff) = payload.strip_prefix(EXIF_ID.as_slice())
        {
            return image::metadata::Orientation::from_exif_chunk(tiff)
                .map_or(Orientation::Upright, ours);
        }
        if marker == SOS {
            break;
        }
        at = next;
    }
    Orientation::Upright
}

/// Ours for the `image` crate's orientation.
const fn ours(turn: image::metadata::Orientation) -> Orientation {
    use image::metadata::Orientation as Theirs;
    match turn {
        Theirs::NoTransforms => Orientation::Upright,
        Theirs::FlipHorizontal => Orientation::FlipHorizontal,
        Theirs::Rotate180 => Orientation::Rotate180,
        Theirs::FlipVertical => Orientation::FlipVertical,
        Theirs::Rotate90 => Orientation::Rotate90,
        Theirs::Rotate270 => Orientation::Rotate270,
        Theirs::Rotate90FlipH => Orientation::Transpose,
        Theirs::Rotate270FlipH => Orientation::Transverse,
    }
}

const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
const APP1: u8 = 0xE1;
const SOS: u8 = 0xDA;
const EXIF_ID: [u8; 6] = *b"Exif\0\0";
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The JPEG segment at `at`: its marker, its payload and where the next one
/// starts. `None` past the end, or on bytes that are no segment.
fn jpeg_segment(bytes: &[u8], at: usize) -> Option<(u8, &[u8], usize)> {
    let [0xFF, marker] = *bytes.get(at..at + 2)? else {
        return None;
    };
    let len = usize::from(u16::from_be_bytes(
        bytes.get(at + 2..at + 4)?.try_into().ok()?,
    ));
    let payload = bytes.get(at + 4..(at + 2).checked_add(len)?)?;
    Some((marker, payload, at + 2 + len))
}

/// Whether a JPEG segment of `marker` carries metadata a stored picture
/// leaves out. Those are the EXIF and XMP of APP1, the APP3 to APP12 and
/// APP15 segments, IPTC in APP13, and a comment. The JFIF header, the ICC
/// profile of APP2 and the colour transform of APP14 decode the picture,
/// and stay.
fn is_metadata(marker: u8) -> bool {
    matches!(marker, 0xE1 | 0xE3..=0xED | 0xEF | 0xFE)
}

/// `bytes` with the metadata a stored picture leaves out removed, the
/// pixels untouched (`PROSE-FEAT-326`).
///
/// A JPEG loses its EXIF, XMP, IPTC and comment segments: the location, the
/// device and the dates they hold. A minimal EXIF segment holding its
/// orientation alone takes their place, so it shows as it was taken. A PNG
/// loses its text, time and EXIF chunks.
///
/// # Errors
///
/// [`MediaFailure::Undecodable`] for a file that is neither a PNG nor a
/// JPEG this reader can walk.
pub fn stripped(bytes: &Payload<MediaBytes>) -> Result<Payload<MediaBytes>, MediaFailure> {
    stripped_in(bytes.as_bytes()).map(Payload::new)
}

/// The kernel of [`stripped`]: a walk of the segments over plain bytes.
fn stripped_in(bytes: &[u8]) -> Result<Vec<u8>, MediaFailure> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        return stripped_png(bytes);
    }
    if !bytes.starts_with(&JPEG_SOI) {
        return Err(MediaFailure::Undecodable);
    }
    let turn = orientation_in(bytes);
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&JPEG_SOI);
    if turn != Orientation::Upright {
        out.extend_from_slice(&orientation_segment(turn));
    }
    let mut at = JPEG_SOI.len();
    loop {
        let (marker, _, next) = jpeg_segment(bytes, at).ok_or(MediaFailure::Undecodable)?;
        if marker == SOS {
            // The scan and every byte after it are the pixels: kept whole.
            out.extend_from_slice(bytes.get(at..).ok_or(MediaFailure::Undecodable)?);
            return Ok(out);
        }
        if !is_metadata(marker) {
            out.extend_from_slice(bytes.get(at..next).ok_or(MediaFailure::Undecodable)?);
        }
        at = next;
    }
}

/// An APP1 segment whose EXIF holds one tag: the orientation.
fn orientation_segment(turn: Orientation) -> [u8; 36] {
    let mut segment = [0u8; 36];
    let head: [u8; 26] = [
        0xFF, APP1, 0x00, 0x22, b'E', b'x', b'i', b'f', 0, 0, // marker, length, identifier
        b'M', b'M', 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08, // big-endian TIFF, IFD at 8
        0x00, 0x01, // one entry
        0x01, 0x12, 0x00, 0x03, // tag 0x0112, type SHORT
        0x00, 0x00, // the high half of the count
    ];
    let [high, low] = turn.exif().to_be_bytes();
    let tail: [u8; 10] = [
        0x00, 0x01, // the low half of the count: one value
        high, low, 0x00, 0x00, // the value, padded
        0x00, 0x00, 0x00, 0x00, // no next IFD
    ];
    segment[..26].copy_from_slice(&head);
    segment[26..].copy_from_slice(&tail);
    segment
}

/// A PNG without its `tEXt`, `zTXt`, `iTXt`, `tIME` and `eXIf` chunks. Each
/// chunk kept is copied with its own CRC, so nothing is recomputed.
fn stripped_png(bytes: &[u8]) -> Result<Vec<u8>, MediaFailure> {
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&PNG_SIGNATURE);
    let mut at = PNG_SIGNATURE.len();
    while at < bytes.len() {
        let len = bytes
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
            .ok_or(MediaFailure::Undecodable)?;
        let end = usize::try_from(len)
            .ok()
            .and_then(|len| at.checked_add(12)?.checked_add(len))
            .filter(|end| *end <= bytes.len())
            .ok_or(MediaFailure::Undecodable)?;
        let kind = bytes.get(at + 4..at + 8).ok_or(MediaFailure::Undecodable)?;
        if !matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" | b"eXIf") {
            out.extend_from_slice(bytes.get(at..end).ok_or(MediaFailure::Undecodable)?);
        }
        if kind == b"IEND" {
            return Ok(out);
        }
        at = end;
    }
    Err(MediaFailure::Undecodable)
}

/// A stored picture as a print draws it: its file, never decoded here.
/// `None` for a file that is neither a PNG nor a JPEG.
#[must_use]
pub fn printed(bytes: &Payload<MediaBytes>) -> Option<PrintedPicture> {
    let format = if bytes.as_bytes().starts_with(&PNG_SIGNATURE) {
        ambre_draw::EncodedFormat::Png
    } else if bytes.as_bytes().starts_with(&JPEG_SOI) {
        ambre_draw::EncodedFormat::Jpeg
    } else {
        return None;
    };
    Some(PrintedPicture {
        format,
        bytes: bytes.clone(),
        size: stored_size(bytes)?,
        orientation: orientation_of(bytes),
    })
}

/// The size `native` is decoded at: itself within [`CACHE_EDGE`], else
/// scaled down to it on its longer edge, its ratio kept.
#[must_use]
pub fn cache_size(native: Size<Px<Image>>) -> Size<Px<Image>> {
    let longer = native.width.max(native.height);
    if longer <= CACHE_EDGE {
        return native;
    }
    let scale = f64::from(CACHE_EDGE.get()) / f64::from(longer.get());
    let fit = |side: Px<Image>| {
        // The result fits the cache edge, so the cast cannot truncate.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a side scaled under the cache edge"
        )]
        let scaled = (f64::from(side.get()) * scale).round() as u32;
        Px::<Image>::new(scaled.max(1))
    };
    Size::new(fit(native.width), fit(native.height))
}

/// The display size a picture of `native` pixels takes when it is inserted:
/// one image pixel to one page pixel, clamped (`PROSE-FEAT-059`).
#[must_use]
pub fn display_of_native(native: Size<Px<Image>>) -> DisplaySize {
    // A pixel count under the source bound is exact in an `f32`.
    #[expect(clippy::cast_precision_loss, reason = "under the source bound")]
    let size = Size::new(
        Px::<Doc>::new(native.width.get() as f32),
        Px::<Doc>::new(native.height.get() as f32),
    );
    clamp_display(size).unwrap_or(Size::new(Px::new(1.0), Px::new(1.0)))
}

/// Why a resize was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeRefusal {
    /// A width or a height of zero, or one that is not a number: a picture
    /// of no area is not a state (`PROSE-FEAT-058`).
    NoArea,
}

/// `size` with each dimension rounded and clamped to [`MAX_DISPLAY_EDGE`]
/// (`PROSE-FEAT-059`).
///
/// # Errors
///
/// [`ResizeRefusal::NoArea`] for a dimension under one pixel once rounded,
/// or one that is not finite (`PROSE-FEAT-058`).
pub fn clamp_display(size: DisplaySize) -> Result<DisplaySize, ResizeRefusal> {
    let side = |px: Px<Doc>| {
        if !px.is_finite() {
            return Err(ResizeRefusal::NoArea);
        }
        let rounded = px.round();
        if rounded < Px::new(1.0) {
            return Err(ResizeRefusal::NoArea);
        }
        Ok(rounded.min(MAX_DISPLAY_EDGE))
    };
    Ok(Size::new(side(size.width)?, side(size.height)?))
}

/// The size a resize asks for, its proportions kept when they are locked
/// (`PROSE-FEAT-214`, `PROSE-FEAT-215`).
///
/// With the aspect locked, the dimension that moved most against `stored`
/// drives. The other follows `ratio`, the decoded picture's width over its
/// height, or the ratio of `stored` when nothing is decoded.
#[must_use]
pub fn locked_size(
    asked: DisplaySize,
    stored: DisplaySize,
    lock: AspectLock,
    decoded: Option<Size<Px<Image>>>,
) -> DisplaySize {
    if lock == AspectLock::Free {
        return asked;
    }
    // A pixel count under the source bound is exact in an `f32`.
    #[expect(clippy::cast_precision_loss, reason = "under the source bound")]
    let ratio = decoded
        .filter(|size| size.height.get() > 0 && size.width.get() > 0)
        .map(|size| size.width.get() as f32 / size.height.get() as f32)
        .or_else(|| {
            (stored.height > Px::ZERO && stored.width > Px::ZERO)
                .then(|| stored.width / stored.height)
        });
    let Some(ratio) = ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0) else {
        return asked;
    };
    let moved_x = (asked.width - stored.width).abs();
    let moved_y = (asked.height - stored.height).abs();
    if moved_x >= moved_y {
        Size::new(asked.width, asked.width / ratio)
    } else {
        Size::new(asked.height * ratio, asked.height)
    }
}

/// The box the layout draws a picture in. Each side is scaled down
/// together when the column is narrower than the stored width. The document
/// keeps the stored size (`PROSE-FEAT-317`).
#[must_use]
pub fn laid_size(stored: DisplaySize, column: Px<Doc>) -> DisplaySize {
    if stored.width <= column || stored.width <= Px::ZERO || column <= Px::ZERO {
        return stored;
    }
    let scale = column / stored.width;
    Size::new(column, stored.height * scale)
}

/// What an image block stores (`ambre-spec-prose-schema.md section 3`).
/// A peer wrote it, so each field reads as its default when it holds no value
/// of its type.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRecord {
    /// The address the media flow gave the bytes, or `None` for a text that
    /// spells no address.
    pub address: Option<Validated<ContentHash>>,
    pub alt: String,
    pub caption: String,
    /// The stored display size. A side that holds nothing reads as zero.
    pub size: DisplaySize,
    pub align: crate::message::ImageAlign,
}

/// The record of the live image block `id`, or `None` for any other block.
pub(crate) fn read_image(tree: &loro::LoroTree, id: &loro::TreeID) -> Option<ImageRecord> {
    use crate::doc::AttrKey;
    use loro::LoroValue;
    if crate::doc::block_type_of(tree, id) != Some(crate::message::BlockType::Image) {
        return None;
    }
    let attrs = crate::doc::attrs_read(tree, id);
    let value = |key: AttrKey| attrs.as_ref().and_then(|attrs| attrs.value(key));
    let text = |key: AttrKey| match value(key) {
        Some(LoroValue::String(text)) => text.to_string(),
        _ => String::new(),
    };
    // A stored side is a whole number of pixels under the display bound, so
    // it is exact in an `f32`.
    #[expect(clippy::cast_precision_loss, reason = "a stored display side")]
    let side = |key: AttrKey| match value(key) {
        Some(LoroValue::I64(n)) => Px::<Doc>::new(n.max(0) as f32),
        Some(LoroValue::Double(n)) if n.is_finite() => Px::<Doc>::new((n as f32).max(0.0)),
        _ => Px::ZERO,
    };
    let align = match value(AttrKey::ImageAlign) {
        Some(LoroValue::I64(n)) => {
            crate::message::ImageAlign::from_byte(u8::try_from(n).unwrap_or(0))
        }
        _ => crate::message::ImageAlign::Block,
    };
    Some(ImageRecord {
        address: Validated::<ContentHash>::parse(text(AttrKey::ContentHash)).ok(),
        alt: text(AttrKey::ImageAlt),
        caption: text(AttrKey::ImageCaption),
        size: Size::new(side(AttrKey::ImageWidth), side(AttrKey::ImageHeight)),
        align,
    })
}

/// One tick of the cache's paint clock: the later a picture was painted, the
/// later it is evicted.
pub enum PaintTick {}

impl ambre_core::SeqKind for PaintTick {
    type IssueRight = ();
    const NAME: &'static str = "PaintTick";
}

/// Where one address stands in a session (`PROSE-FEAT-207`). An address the
/// cache holds no entry for is unasked.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    /// Asked for, waiting on the Sync worker. Never evicted
    /// (`PROSE-FEAT-055`).
    Loading,
    Ready(Arc<DecodedPicture>),
    /// Failed for good: never asked again without a gesture
    /// (`PROSE-FEAT-057`).
    Failed(MediaFailure),
}

/// The decoded pictures of one document, held under a cap in decoded bytes
/// (`PROSE-FEAT-054`). It lives on the Logic thread and dies with the
/// session: nothing of it reaches the document (`PROSE-FEAT-207`).
#[derive(Debug, Default)]
pub struct PictureCache {
    entries: HashMap<Validated<ContentHash>, (Entry, Seq<PaintTick>)>,
    held: Count<Octets>,
    clock: Seq<PaintTick>,
}

/// What [`PictureCache::want`] decided for one address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// Unasked until now: the caller asks the Sync worker for it.
    Fetch,
    /// Already asked, decoded or failed: nothing to ask.
    Held,
}

impl PictureCache {
    /// The decoded bytes held now.
    #[must_use]
    pub fn held(&self) -> Count<Octets> {
        self.held
    }

    /// What the view shows for `address`.
    #[must_use]
    pub fn shown(&self, address: &Validated<ContentHash>) -> PictureShown {
        match self.entries.get(address).map(|(entry, _)| entry) {
            Some(Entry::Ready(picture)) => PictureShown::Ready(Arc::clone(picture)),
            Some(Entry::Failed(failure)) => PictureShown::Failed(*failure),
            Some(Entry::Loading) | None => PictureShown::Waiting,
        }
    }

    /// The decoded picture of `address`, if the cache holds it.
    #[must_use]
    pub fn decoded(&self, address: &Validated<ContentHash>) -> Option<Arc<DecodedPicture>> {
        match self.entries.get(address) {
            Some((Entry::Ready(picture), _)) => Some(Arc::clone(picture)),
            _ => None,
        }
    }

    /// Whether `address` waits on the Sync worker.
    #[must_use]
    pub fn is_loading(&self, address: &Validated<ContentHash>) -> bool {
        matches!(self.entries.get(address), Some((Entry::Loading, _)))
    }

    /// Mark `address` painted now, and say whether it must be asked for. An
    /// unasked address turns `Loading`. A failed one stays failed
    /// (`PROSE-FEAT-057`).
    pub fn want(&mut self, address: &Validated<ContentHash>) -> Want {
        self.clock = self.clock.next();
        let now = self.clock;
        match self.entries.get_mut(address) {
            Some((_, painted)) => {
                *painted = now;
                Want::Held
            }
            None => {
                self.entries.insert(address.clone(), (Entry::Loading, now));
                Want::Fetch
            }
        }
    }

    /// Hold `picture` for `address`, then evict until the total fits
    /// (`PROSE-FEAT-223`, `PROSE-FEAT-286`). Returns the addresses evicted,
    /// which are unasked again: the caller tells the view to let go of them.
    pub fn insert(
        &mut self,
        address: &Validated<ContentHash>,
        picture: Arc<DecodedPicture>,
    ) -> Vec<Validated<ContentHash>> {
        self.clock = self.clock.next();
        let held = picture.held();
        if let Some((Entry::Ready(old), _)) = self.entries.get(address) {
            self.held = self.held.minus(old.held());
        }
        self.entries
            .insert(address.clone(), (Entry::Ready(picture), self.clock));
        self.held = self.held.plus(held);
        self.evict(address)
    }

    /// Record that `address` failed for good.
    pub fn fail(&mut self, address: &Validated<ContentHash>, failure: MediaFailure) {
        self.clock = self.clock.next();
        if let Some((Entry::Ready(old), _)) = self.entries.get(address) {
            self.held = self.held.minus(old.held());
        }
        self.entries
            .insert(address.clone(), (Entry::Failed(failure), self.clock));
    }

    /// Put `address` back to unasked, whatever it was (`PROSE-FEAT-221`).
    /// Another address is not touched (`PROSE-FEAT-222`).
    pub fn forget(&mut self, address: &Validated<ContentHash>) {
        if let Some((Entry::Ready(old), _)) = self.entries.remove(address) {
            self.held = self.held.minus(old.held());
        }
    }

    /// Drop the least recently painted decoded pictures until the total fits
    /// the cap. A transfer in flight is never chosen, and neither is `kept`,
    /// the picture just held: one entry stands even over the cap
    /// (`PROSE-FEAT-055`, `PROSE-FEAT-224`).
    fn evict(&mut self, kept: &Validated<ContentHash>) -> Vec<Validated<ContentHash>> {
        let mut evicted = Vec::new();
        while self.held > CACHE_CAP {
            let victim = self
                .entries
                .iter()
                .filter(|(address, (entry, _))| {
                    *address != kept && matches!(entry, Entry::Ready(_))
                })
                .min_by_key(|(_, (_, painted))| *painted)
                .map(|(address, _)| address.clone());
            let Some(victim) = victim else {
                break;
            };
            self.forget(&victim);
            evicted.push(victim);
        }
        evicted
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn address(digit: char) -> Validated<ContentHash> {
        Validated::<ContentHash>::parse(digit.to_string().repeat(64)).expect("a hex digit")
    }

    /// A decoded picture of `octets` bytes.
    fn picture(octets: usize) -> Arc<DecodedPicture> {
        Arc::new(DecodedPicture {
            pixels: Payload::new(vec![0; octets]),
            size: Size::new(Px::new(1), Px::new(1)),
        })
    }

    fn cap() -> usize {
        usize::try_from(CACHE_CAP.get()).expect("the cap fits a usize")
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::new(width, height)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("a PNG encodes");
        bytes
    }

    /// `PROSE-FEAT-054`: the cap counts decoded bytes, not entries. Two
    /// pictures that fit stay; a third that tips the total over evicts.
    #[test]
    fn the_cap_counts_decoded_bytes_not_entries() {
        let mut cache = PictureCache::default();
        let third = cap() / 3;
        for digit in ['a', 'b'] {
            cache.want(&address(digit));
            assert!(cache.insert(&address(digit), picture(third)).is_empty());
        }
        cache.want(&address('c'));
        assert!(cache.insert(&address('c'), picture(third)).is_empty());
        cache.want(&address('d'));
        let evicted = cache.insert(&address('d'), picture(third));
        assert_eq!(evicted, [address('a')], "one entry leaves, by bytes");
        assert!(cache.held() <= CACHE_CAP);
    }

    /// `PROSE-FEAT-223`, `PROSE-FEAT-286`: the least recently painted goes
    /// first, and eviction stops once the total fits.
    #[test]
    fn eviction_takes_the_least_recently_painted_and_stops_when_it_fits() {
        let mut cache = PictureCache::default();
        let half = cap() / 2;
        cache.want(&address('a'));
        cache.insert(&address('a'), picture(half - 1));
        cache.want(&address('b'));
        cache.insert(&address('b'), picture(half - 1));
        // `a` is painted again, so `b` is now the older.
        cache.want(&address('a'));
        cache.want(&address('c'));
        let evicted = cache.insert(&address('c'), picture(3));
        assert_eq!(evicted, [address('b')]);
        assert!(matches!(cache.shown(&address('a')), PictureShown::Ready(_)));
    }

    /// `PROSE-FEAT-224`: one entry larger than the whole cap is kept.
    #[test]
    fn one_entry_stands_even_over_the_cap() {
        let mut cache = PictureCache::default();
        cache.want(&address('a'));
        assert!(cache.insert(&address('a'), picture(cap() + 1)).is_empty());
        assert!(matches!(cache.shown(&address('a')), PictureShown::Ready(_)));
    }

    /// `PROSE-FEAT-055`: a transfer in flight is never evicted, however old.
    #[test]
    fn a_transfer_in_flight_is_never_evicted() {
        let mut cache = PictureCache::default();
        cache.want(&address('a'));
        cache.want(&address('b'));
        cache.insert(&address('b'), picture(cap() / 2 + 1));
        cache.want(&address('c'));
        let evicted = cache.insert(&address('c'), picture(cap() / 2 + 1));
        assert_eq!(evicted, [address('b')]);
        assert!(cache.is_loading(&address('a')));
    }

    /// `PROSE-FEAT-057`: a failed address is not asked again by a paint.
    #[test]
    fn a_failed_picture_is_not_asked_again_by_a_paint() {
        let mut cache = PictureCache::default();
        assert_eq!(cache.want(&address('a')), Want::Fetch);
        cache.fail(&address('a'), MediaFailure::Unavailable);
        assert_eq!(cache.want(&address('a')), Want::Held);
        assert_eq!(
            cache.shown(&address('a')),
            PictureShown::Failed(MediaFailure::Unavailable)
        );
        cache.forget(&address('a'));
        assert_eq!(cache.want(&address('a')), Want::Fetch);
    }

    /// `PROSE-FEAT-058`, `PROSE-FEAT-059`: zero is refused, a huge size is
    /// clamped, and a fraction is rounded.
    #[test]
    fn a_zero_side_is_refused_and_a_huge_one_clamped() {
        let size = |w: f32, h: f32| Size::new(Px::<Doc>::new(w), Px::<Doc>::new(h));
        assert_eq!(clamp_display(size(0.0, 10.0)), Err(ResizeRefusal::NoArea));
        assert_eq!(clamp_display(size(10.0, 0.2)), Err(ResizeRefusal::NoArea));
        assert_eq!(
            clamp_display(size(f32::NAN, 10.0)),
            Err(ResizeRefusal::NoArea)
        );
        assert_eq!(clamp_display(size(9_000.0, 10.4)), Ok(size(4096.0, 10.0)));
    }

    /// `PROSE-FEAT-214`, `PROSE-FEAT-215`: the dimension that moved most
    /// drives, the other follows the decoded ratio, else the stored one.
    #[test]
    fn a_locked_resize_follows_the_side_that_moved_most() {
        let size = |w: f32, h: f32| Size::new(Px::<Doc>::new(w), Px::<Doc>::new(h));
        let stored = size(200.0, 100.0);
        let decoded = Some(Size::new(Px::<Image>::new(400), Px::<Image>::new(100)));
        // The width moved by 100, the height by 10: the width drives.
        assert_eq!(
            locked_size(size(300.0, 110.0), stored, AspectLock::Locked, decoded),
            size(300.0, 75.0)
        );
        // The height moved most, the stored ratio follows with no decode.
        assert_eq!(
            locked_size(size(210.0, 200.0), stored, AspectLock::Locked, None),
            size(400.0, 200.0)
        );
        assert_eq!(
            locked_size(size(210.0, 200.0), stored, AspectLock::Free, None),
            size(210.0, 200.0)
        );
    }

    /// `PROSE-FEAT-314`, `PROSE-FEAT-315`: a header past the source bound is
    /// refused before any decode. A picture past the cache edge is held at
    /// it, its ratio kept.
    #[test]
    fn a_source_past_the_bound_is_refused_and_a_large_one_scaled_down() {
        assert_eq!(
            decode_here(&media(&png(8193, 1))).map(|p| p.size),
            Err(MediaFailure::Undecodable)
        );
        let decoded = decode_here(&media(&png(4096, 1024))).expect("a PNG decodes");
        assert_eq!(decoded.size, Size::new(Px::new(2048), Px::new(512)));
        assert_eq!(decoded.held(), Count::<Octets>::new(2048 * 512 * 4));
        let small = decode_here(&media(&png(3, 2))).expect("a PNG decodes");
        assert_eq!(small.size, Size::new(Px::new(3), Px::new(2)));
        assert_eq!(
            decode_here(&media(b"not a picture")),
            Err(MediaFailure::Undecodable).map(|p: DecodedPicture| p)
        );
    }

    /// `PROSE-FEAT-317`: a picture wider than the column is drawn at the
    /// column's width, its ratio kept.
    #[test]
    fn a_picture_wider_than_the_column_is_scaled_to_it() {
        let size = |w: f32, h: f32| Size::new(Px::<Doc>::new(w), Px::<Doc>::new(h));
        assert_eq!(
            laid_size(size(1248.0, 400.0), Px::new(624.0)),
            size(624.0, 200.0)
        );
        assert_eq!(
            laid_size(size(300.0, 400.0), Px::new(624.0)),
            size(300.0, 400.0)
        );
    }

    /// A JPEG of `width` by `height` pixels, written by the encoder.
    pub(crate) fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let picture = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x * 40) as u8, (y * 40) as u8, 128])
        });
        image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut bytes), 90)
            .encode_image(&picture)
            .expect("a JPEG encodes");
        bytes
    }

    /// One JPEG segment of `marker` holding `payload`.
    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let len = u16::try_from(payload.len() + 2).expect("a short segment");
        let mut out = vec![0xFF, marker];
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// An EXIF block, big-endian: IFD0 holds the orientation, the camera's
    /// make and a pointer to a GPS IFD, which holds a latitude reference.
    fn exif_with_gps(orientation: u16) -> Vec<u8> {
        let mut tiff = b"MM\x00\x2a\x00\x00\x00\x08".to_vec();
        // IFD0 at 8: three entries.
        tiff.extend_from_slice(&3u16.to_be_bytes());
        let entry = |tag: u16, kind: u16, count: u32, value: [u8; 4]| {
            let mut e = tag.to_be_bytes().to_vec();
            e.extend_from_slice(&kind.to_be_bytes());
            e.extend_from_slice(&count.to_be_bytes());
            e.extend_from_slice(&value);
            e
        };
        let [o0, o1] = orientation.to_be_bytes();
        tiff.extend(entry(0x0112, 3, 1, [o0, o1, 0, 0]));
        tiff.extend(entry(0x010F, 2, 4, *b"ACME"));
        // The GPS IFD sits after IFD0: 8 + 2 + 3 * 12 + 4 = 50.
        tiff.extend(entry(0x8825, 4, 1, 50u32.to_be_bytes()));
        tiff.extend_from_slice(&0u32.to_be_bytes());
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend(entry(0x0001, 2, 2, *b"N\x00\x00\x00"));
        tiff.extend_from_slice(&0u32.to_be_bytes());
        let mut payload = b"Exif\x00\x00".to_vec();
        payload.extend(tiff);
        payload
    }

    /// A JPEG that carries a location, a camera, an XMP packet and a
    /// comment beside its pixels, turned a quarter clockwise.
    pub(crate) fn jpeg_with_metadata() -> Vec<u8> {
        let plain = jpeg(8, 4);
        let mut out = plain[..2].to_vec();
        out.extend(segment(0xE1, &exif_with_gps(6)));
        out.extend(segment(
            0xE1,
            b"http://ns.adobe.com/xap/1.0/\x00<x:xmpmeta>secret place</x:xmpmeta>",
        ));
        out.extend(segment(0xED, b"Photoshop 3.0\x00IPTC city"));
        out.extend(segment(0xFE, b"a secret comment"));
        out.extend_from_slice(&plain[2..]);
        out
    }

    /// `bytes` as the media flow stores them.
    pub(crate) fn media(bytes: &[u8]) -> Payload<MediaBytes> {
        Payload::new(bytes.to_vec())
    }

    pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    /// `PROSE-FEAT-326`: a stored JPEG carries none of the metadata its file
    /// held: no location, no camera, no XMP, no IPTC and no comment.
    #[test]
    fn a_stored_picture_carries_no_exif_location() {
        let source = jpeg_with_metadata();
        assert!(contains(&source, b"ACME") && contains(&source, &0x8825u16.to_be_bytes()));
        let stored = stripped(&media(&source)).expect("a JPEG strips");
        for left_out in [
            b"ACME".as_slice(),
            b"secret place",
            b"IPTC city",
            b"a secret comment",
            b"N\x00\x00\x00",
        ] {
            assert!(!contains(stored.as_bytes(), left_out), "{left_out:?} left");
        }
        assert!(
            !contains(stored.as_bytes(), &[0x88, 0x25]),
            "no GPS pointer"
        );
    }

    /// `PROSE-FEAT-327`: stripping leaves the scan, the pixels, byte for
    /// byte.
    #[test]
    fn stripping_leaves_the_pixels_byte_for_byte() {
        let source = jpeg_with_metadata();
        let stored = stripped(&media(&source)).expect("a JPEG strips");
        let scan = |bytes: &[u8]| {
            let at = bytes
                .windows(2)
                .position(|w| w == [0xFF, SOS])
                .expect("a scan");
            bytes[at..].to_vec()
        };
        assert_eq!(scan(stored.as_bytes()), scan(&source));
        let png_source = {
            let mut plain = Vec::new();
            image::RgbaImage::from_pixel(2, 2, image::Rgba([7; 4]))
                .write_to(&mut Cursor::new(&mut plain), image::ImageFormat::Png)
                .expect("a PNG encodes");
            // A text chunk after the 33 bytes of signature and header.
            let body = b"tEXtAuthor\x00a secret name";
            let crc = body.iter().fold(0xFFFF_FFFFu32, |mut crc, byte| {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
                crc
            }) ^ 0xFFFF_FFFF;
            let mut bytes = plain[..33].to_vec();
            bytes.extend_from_slice(&u32::try_from(body.len() - 4).expect("short").to_be_bytes());
            bytes.extend_from_slice(body);
            bytes.extend_from_slice(&crc.to_be_bytes());
            bytes.extend_from_slice(&plain[33..]);
            bytes
        };
        assert!(
            decode_here(&media(&png_source)).is_ok(),
            "the chunk is well formed"
        );
        assert!(contains(&png_source, b"a secret name"));
        let png_stored = stripped(&media(&png_source)).expect("a PNG strips");
        assert!(!contains(png_stored.as_bytes(), b"a secret name"));
        let decoded = decode_here(&png_stored).expect("the stripped PNG decodes");
        assert_eq!(decoded.pixels.as_bytes(), &[7; 16]);
    }

    /// `PROSE-FEAT-328`: the stored picture keeps its orientation alone, so
    /// it shows as it was taken: on its side, as the file asked.
    #[test]
    fn a_stored_picture_keeps_its_orientation() {
        let stored = stripped(&media(&jpeg_with_metadata())).expect("a JPEG strips");
        assert_eq!(orientation_of(&stored), Orientation::Rotate90);
        assert_eq!(native_size(&stored), Ok(Size::new(Px::new(4), Px::new(8))));
        let decoded = decode_here(&stored).expect("the stored JPEG decodes");
        assert_eq!(decoded.size, Size::new(Px::new(4), Px::new(8)));
        let upright = stripped(&media(&jpeg(8, 4))).expect("a JPEG strips");
        assert_eq!(orientation_of(&upright), Orientation::Upright);
        assert!(
            !contains(upright.as_bytes(), b"Exif"),
            "an upright picture keeps no EXIF"
        );
    }
}
