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
    ContentHash, Count, Doc, Image, Octets, Payload, Px, RgbaPixels, Seq, Size, Validated,
};
use image::{GenericImageView, ImageReader, Limits};

use crate::message::{AspectLock, DecodedPicture, DisplaySize, MediaFailure, PictureShown};

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

/// Decode `bytes` into a picture of at most [`CACHE_EDGE`] a side.
///
/// The header is read first and held to [`MAX_SOURCE_EDGE`], so a file that
/// declares a huge size costs its header alone. The decoder carries the same
/// bound, so a header that lies is refused too.
///
/// # Errors
///
/// [`MediaFailure::Undecodable`] for a format this build does not decode, a
/// header past the bound, or a broken file.
pub fn decode(bytes: &[u8]) -> Result<DecodedPicture, MediaFailure> {
    let native = native_size(bytes)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE.get());
    limits.max_image_height = Some(MAX_SOURCE_EDGE.get());
    limits.max_alloc = Some(MAX_DECODE_ALLOC.get());
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| MediaFailure::Undecodable)?;
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| MediaFailure::Undecodable)?;
    let size = cache_size(native);
    // The scale runs on the file's own samples, so a JPEG is never expanded
    // to four bytes a pixel at its full size.
    let scaled = if size == native {
        decoded
    } else {
        decoded.resize_exact(
            size.width.get(),
            size.height.get(),
            image::imageops::FilterType::Triangle,
        )
    };
    let (width, height) = scaled.dimensions();
    Ok(DecodedPicture {
        pixels: Payload::<RgbaPixels>::new(scaled.into_rgba8().into_raw()),
        size: Size::new(Px::new(width), Px::new(height)),
    })
}

/// The format a file of a package names for `bytes`, read off their
/// signature. `None` for a format no office file holds.
#[must_use]
pub fn file_format(bytes: &[u8]) -> Option<ambre_prose_form::ImageFormat> {
    use ambre_prose_form::ImageFormat as Format;
    Some(match image::guess_format(bytes).ok()? {
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
pub fn native_size(bytes: &[u8]) -> Result<Size<Px<Image>>, MediaFailure> {
    let (width, height) = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| MediaFailure::Undecodable)?
        .into_dimensions()
        .map_err(|_| MediaFailure::Undecodable)?;
    let size = Size::new(Px::new(width), Px::new(height));
    let zero = Px::<Image>::new(0);
    if size.width == zero
        || size.height == zero
        || size.width > MAX_SOURCE_EDGE
        || size.height > MAX_SOURCE_EDGE
    {
        return Err(MediaFailure::Undecodable);
    }
    Ok(size)
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
mod tests {
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
            decode(&png(8193, 1)).map(|p| p.size),
            Err(MediaFailure::Undecodable)
        );
        let decoded = decode(&png(4096, 1024)).expect("a PNG decodes");
        assert_eq!(decoded.size, Size::new(Px::new(2048), Px::new(512)));
        assert_eq!(decoded.held(), Count::<Octets>::new(2048 * 512 * 4));
        let small = decode(&png(3, 2)).expect("a PNG decodes");
        assert_eq!(small.size, Size::new(Px::new(3), Px::new(2)));
        assert_eq!(
            decode(b"not a picture"),
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
}
