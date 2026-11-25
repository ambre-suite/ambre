// ambre-canvas/src/render/atlas.rs
// ============================================================================
// GlyphAtlas: Swash rasterization into two 2048x2048 GPU textures. An R8Unorm
// coverage-mask atlas serves regular text glyphs, a parallel Rgba8Unorm atlas
// full-color glyphs (color emoji). A shelf packer and the per-glyph UV/bearing
// cache complete it. Batched CPU->GPU uploads once per frame.
//
// Color glyphs (for example the CBDT bitmap Noto Color Emoji) come back from
// Swash as `Content::Color` (straight-alpha RGBA) and are packed into the color
// atlas. Everything else stays on the alpha-mask path with no extra memory cost.
// Which atlas a glyph lives in is recorded on its `AtlasSlot::atlas`, so the
// draw path can route it to the matching pipeline. Keeping two atlases (rather
// than one RGBA atlas for all glyphs) avoids paying 4x texture memory for text,
// which dominates the working set.
// ============================================================================

use rustc_hash::FxHashMap;

use ambre_core::{
    Count, CountsSince, Css, Device, NarrowCount, Point, Px, Scale, Seq, SeqKind, Size, Texel,
};

use crate::space::{GlyphRaster, Parley, RasterKey, TexelShift, Uv};

// ============================================================================
// GlyphAtlas
// ============================================================================

/// The side of both atlases.
const ATLAS_SIZE: Px<Texel> = Px::<Texel>::new(2048);
/// The largest em a glyph is rasterized at, in device pixels: half the atlas
/// side. A glyph box up to twice its em then still packs. A larger size is
/// rasterized at this one and drawn scaled up (`CANVAS-087`).
const RASTER_CAP: Px<GlyphRaster> = Px::<GlyphRaster>::new((ATLAS_SIZE.get() / 2) as f32);
// 1-pixel padding between glyphs prevents bilinear bleed at atlas boundaries.
const ATLAS_PAD: Px<Texel> = Px::<Texel>::new(1);

/// The frames the atlas ages its glyphs by: the clock `begin_frame` steps,
/// and the frame a glyph was last drawn on.
pub(super) enum AtlasFrames {}

impl SeqKind for AtlasFrames {
    type IssueRight = ();
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("AtlasFrame");
}

impl CountsSince for AtlasFrames {
    type Count = FramesUnused;
}

/// The atlas frames since a glyph was last drawn.
pub(super) enum FramesUnused {}

impl NarrowCount for FramesUnused {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("FramesUnused");
}

/// A glyph's box in its atlas, as a sampler reads it: the corner nearest the
/// origin of the texture, and the opposite one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UvBox {
    /// The top-left corner.
    pub min: Point<Px<Uv>>,
    /// The bottom-right corner.
    pub max: Point<Px<Uv>>,
}

impl UvBox {
    /// The whole texture, as a picture samples it.
    pub const WHOLE: Self = Self {
        min: Point::new(Px::<Uv>::new(0.0), Px::<Uv>::new(0.0)),
        max: Point::new(Px::<Uv>::new(1.0), Px::<Uv>::new(1.0)),
    };

    /// The four corners, `[tl, tr, br, bl]`, the order a quad lists its own.
    pub fn corners(self) -> [Point<Px<Uv>>; 4] {
        let (min, max) = (self.min, self.max);
        [min, Point::new(max.x, min.y), max, Point::new(min.x, max.y)]
    }

    /// The box of `size` texels at `at`, in an atlas of side `side`.
    fn of(at: Point<Px<Texel>>, size: Size<Px<Texel>>, side: Px<Texel>) -> Self {
        // The ratio of a texel to the side: a texel length over a texel length.
        let inv = 1.0 / side.get() as f32;
        let uv = |texels: Px<Texel>| Px::<Uv>::new(texels.get() as f32 * inv);
        Self {
            min: Point::new(uv(at.x), uv(at.y)),
            max: Point::new(uv(at.x + size.width), uv(at.y + size.height)),
        }
    }
}

/// The font a glyph is drawn from: the id of its registered blob, stable for
/// as long as the blob lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct FontId(u64);

impl FontId {
    /// The font of the blob whose id is `blob_id`.
    pub(super) const fn of_blob(blob_id: u64) -> Self {
        Self(blob_id)
    }
}

/// A glyph's index in its font, as the shaper names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(u16);

impl GlyphId {
    /// The glyph at `index` in its font.
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    /// The index Swash renders.
    const fn get(self) -> u16 {
        self.0
    }
}

/// The size, in device pixels, that a glyph run is rasterized at. Its layout
/// gives it `font_size`, in the units of that layout. The run is drawn under a
/// transform of `scale`, on a screen of `density`.
///
/// A walk is laid out in document units and drawn under a zoom. Its glyphs get
/// a bitmap of the size they cover on the screen, never a stretched one
/// (`CANVAS-074`). The atlas keys a raster by this size, so a density change
/// never draws a raster made for the previous screen (`CANVAS-034`).
pub(crate) fn raster_of_layout(
    font_size: Px<Parley>,
    scale: f64,
    density: Scale<Css, Device>,
) -> Px<GlyphRaster> {
    Px::new((f64::from(font_size.get()) * scale) as f32 * density.get() as f32)
}

/// The size the bitmap is made at: `size`, or [`RASTER_CAP`] past it.
fn capped(size: Px<GlyphRaster>) -> Px<GlyphRaster> {
    size.min(RASTER_CAP)
}

/// The device pixels one texel of the bitmap covers: one up to
/// [`RASTER_CAP`], and the ratio of `size` to the cap past it.
fn texel_scale(size: Px<GlyphRaster>) -> f32 {
    (size / RASTER_CAP).max(1.0)
}

/// The key of a raster `size` device pixels high: the size quantized to a
/// hundredth of a device pixel, so it can be hashed.
fn raster_key(size: Px<GlyphRaster>) -> Px<RasterKey> {
    Px::new((size.get() * 100.0) as u32)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct GlyphKey {
    font: FontId,
    glyph: GlyphId,
    size: Px<RasterKey>,
}

impl GlyphKey {
    /// The key of a raster at `raster_size`, the size in device pixels. A
    /// raster made for one density is never found for another (`CANVAS-034`).
    pub(super) fn new(font: FontId, glyph: GlyphId, raster_size: Px<GlyphRaster>) -> Self {
        Self {
            font,
            glyph,
            size: raster_key(raster_size),
        }
    }
}

/// Retained bitmap for a packed glyph, kept so the atlas can repack its working
/// set on overflow (LRU compaction) without re-rasterizing through Swash.
/// Invisible glyphs (space) have no bitmap. Whether the bytes are R8 coverage
/// or RGBA color is carried by the owning slot's `atlas`. The compaction
/// path reads it to re-queue into the matching atlas.
struct GlyphBitmap {
    size: Size<Px<Texel>>,
    data: Vec<u8>,
}

/// A cache entry: the current atlas slot, the frame it was last drawn on (LRU
/// compaction), and the retained bitmap. `None` marks a zero-size glyph.
struct CachedGlyph {
    slot: AtlasSlot,
    last_used: Seq<AtlasFrames>,
    bitmap: Option<GlyphBitmap>,
}

/// UV rect and pixel-space bearing for one atlas entry.
#[derive(Clone, Copy)]
pub struct AtlasSlot {
    /// The box of the bitmap in its atlas.
    pub uv: UvBox,
    /// The size of the bitmap.
    pub size: Size<Px<Texel>>,
    /// Horizontal offset from the glyph origin to the bitmap left edge.
    pub bearing_left: Px<TexelShift>,
    /// Vertical offset from the glyph origin (baseline) to the bitmap top.
    /// Positive = above baseline. Apply as: `screen_y = origin_y - bearing_top`.
    pub bearing_top: Px<TexelShift>,
    /// The atlas this glyph lives in. Routes the draw to the matching
    /// pipeline.
    pub atlas: AtlasKind,
    /// The device pixels one texel covers: 1, or more for a glyph rasterized
    /// at the cap and drawn scaled up (`CANVAS-087`). The pixel lengths above
    /// are in texels.
    pub texel_scale: f32,
}

ambre_core::define_flag! {
    /// Which atlas a glyph lives in (`CANVAS-019`).
    pub enum AtlasKind {
        /// The R8 coverage-mask atlas, drawn in the run's colour.
        Mask,
        /// The RGBA colour atlas (colour emoji), drawn untinted.
        Color,
    }
}

struct ShelfPacker {
    cur_x: Px<Texel>,
    /// The top of the current shelf.
    cur_y: Px<Texel>,
    /// The tallest glyph on the current shelf.
    shelf_h: Px<Texel>,
    atlas_size: Px<Texel>,
}

impl ShelfPacker {
    fn new(atlas_size: Px<Texel>) -> Self {
        Self {
            cur_x: Px::ZERO,
            cur_y: Px::ZERO,
            shelf_h: Px::ZERO,
            atlas_size,
        }
    }

    fn reset(&mut self) {
        self.cur_x = Px::ZERO;
        self.cur_y = Px::ZERO;
        self.shelf_h = Px::ZERO;
    }

    /// Try to allocate a region of `size` texels.
    /// Returns the top-left texel on success, or `None` if the
    /// atlas is full (caller should reset the atlas and retry).
    fn allocate(&mut self, size: Size<Px<Texel>>) -> Option<Point<Px<Texel>>> {
        let w_padded = size.width + ATLAS_PAD;
        // Advance to next shelf if this glyph doesn't fit horizontally.
        if self.cur_x + w_padded > self.atlas_size {
            self.cur_y += self.shelf_h + ATLAS_PAD;
            self.cur_x = Px::ZERO;
            self.shelf_h = Px::ZERO;
        }
        // Check vertical overflow.
        if self.cur_y + size.height > self.atlas_size {
            return None;
        }
        let at = Point::new(self.cur_x, self.cur_y);
        self.cur_x += w_padded;
        self.shelf_h = self.shelf_h.max(size.height);
        Some(at)
    }
}

/// CPU + GPU glyph atlas.
///
/// Glyphs are rasterized via Swash into a 2048x2048 R8Unorm GPU texture.
/// Pending CPU-to-GPU uploads are batched and flushed once per frame.
pub struct GlyphAtlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Parallel RGBA color atlas for `Content::Color` glyphs (color emoji).
    pub color_texture: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    /// Everything the atlas decides without a device: the packing, the
    /// eviction order and the queued uploads.
    glyphs: GlyphCache,
    scale_cx: swash::scale::ScaleContext,
}

/// A rectangle of texels in an atlas: its top-left corner and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TexelRect {
    x: Px<Texel>,
    y: Px<Texel>,
    size: Size<Px<Texel>>,
}

/// One queued texture upload: the bytes and the texels they fill.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Upload {
    rect: TexelRect,
    bytes: Vec<u8>,
}

/// One glyph as the rasterizer returned it, before it is packed.
pub(super) struct Raster {
    /// The bitmap size; zero on an edge for an invisible glyph.
    pub(super) size: Size<Px<Texel>>,
    /// Horizontal offset from the glyph origin to the bitmap's left edge.
    pub(super) left: Px<TexelShift>,
    /// Vertical offset from the baseline to the bitmap's top edge.
    pub(super) top: Px<TexelShift>,
    /// Whether the bytes are RGBA colour rather than an R8 coverage mask.
    pub(super) atlas: AtlasKind,
    /// The bitmap, one byte per pixel for a mask and four for colour.
    pub(super) data: Vec<u8>,
}

/// The CPU half of the glyph atlas. It owns the slot of every cached glyph,
/// the two shelf packers, the eviction order and the uploads still to send.
/// No device is named here, so a host test drives it whole.
pub(super) struct GlyphCache {
    /// The side of both atlases.
    size: Px<Texel>,
    cache: FxHashMap<GlyphKey, CachedGlyph>,
    packer: ShelfPacker,
    /// Independent shelf packer for the color atlas.
    color_packer: ShelfPacker,
    /// Queued R8 texture uploads.
    pending: Vec<Upload>,
    /// Queued RGBA color texture uploads.
    color_pending: Vec<Upload>,
    /// The color atlas is allocated lazily: a full 2048x2048 RGBA texture is 16
    /// MB, wasted on the many documents with no color emoji. Until the first
    /// color glyph appears the texture is a 1x1 placeholder. `color_ready` gates
    /// packing, `color_wanted` latches the first sighting so the renderer can
    /// grow the texture (and rebuild its bind group) on the next frame.
    color_ready: bool,
    color_wanted: bool,
    /// Monotonic frame counter, bumped in `begin_frame`. A glyph drawn this
    /// frame has `last_used == frame`; compaction keeps that working set.
    frame: Seq<AtlasFrames>,
    /// Set when an allocation overflowed mid-frame. Compaction is deferred to
    /// the next `begin_frame` so it never moves a glyph whose UVs were already
    /// baked into this frame's vertex buffer.
    needs_compact: bool,
}

impl GlyphCache {
    /// An empty cache over two atlases of `size` x `size` pixels.
    pub(super) fn new(size: Px<Texel>) -> Self {
        Self {
            size,
            cache: FxHashMap::default(),
            packer: ShelfPacker::new(size),
            color_packer: ShelfPacker::new(size),
            pending: Vec::new(),
            color_pending: Vec::new(),
            color_ready: false,
            color_wanted: false,
            frame: Seq::ZERO,
            needs_compact: false,
        }
    }

    /// Whether the colour atlas must be grown to full size now. Only a colour
    /// glyph already seen asks for it, and only once.
    pub(super) fn color_atlas_due(&self) -> bool {
        self.color_wanted && !self.color_ready
    }

    /// Advance the frame clock that ages glyphs, and run a compaction the
    /// previous frame deferred.
    pub(super) fn begin_frame(&mut self) {
        self.frame = self.frame.next();
        if self.needs_compact {
            self.compact();
            self.needs_compact = false;
        }
    }

    /// Repack the recently-used glyphs into a fresh atlas, dropping the cold
    /// ones. Called at frame start after an overflow. The old full-clear
    /// threw away every glyph, including the ones still on screen, causing a
    /// blank flash. Instead we keep glyphs touched within the last
    /// `KEEP_FRAMES` frames, re-pack them from their retained bitmaps (no
    /// re-rasterization), and re-queue those bitmaps for upload.
    fn compact(&mut self) {
        // Keep the frame that overflowed and the one before it, so glyphs that
        // flicker in and out across a couple of frames are not thrashed. This
        // runs at the start of a frame, which has drawn nothing yet.
        const KEEP_FRAMES: Count<FramesUnused> = Count::new(2);
        let frame = self.frame;

        self.packer.reset();
        self.color_packer.reset();
        self.pending.clear();
        self.color_pending.clear();

        let mut survivors: FxHashMap<GlyphKey, CachedGlyph> = FxHashMap::default();
        // Re-pack in descending recency so the hottest glyphs are placed first
        // and are the least likely to be evicted if the working set is tight.
        let mut entries: Vec<(GlyphKey, CachedGlyph)> = self
            .cache
            .drain()
            .filter(|(_, c)| frame.since(c.last_used) <= KEEP_FRAMES)
            .collect();
        entries.sort_by_key(|(_, c)| std::cmp::Reverse(c.last_used));

        for (key, mut cached) in entries {
            match &cached.bitmap {
                // Zero-size (invisible) glyph: no atlas space, keep as-is.
                None => {
                    survivors.insert(key, cached);
                }
                Some(bmp) => {
                    // Re-pack into the atlas matching the glyph's format.
                    let (packer, pending) = if cached.slot.atlas == AtlasKind::Color {
                        (&mut self.color_packer, &mut self.color_pending)
                    } else {
                        (&mut self.packer, &mut self.pending)
                    };
                    let Some(at) = packer.allocate(bmp.size) else {
                        // Working set itself overflows: drop the rest.
                        continue;
                    };
                    cached.slot.uv = UvBox::of(at, bmp.size, self.size);
                    pending.push(Upload {
                        rect: TexelRect {
                            x: at.x,
                            y: at.y,
                            size: bmp.size,
                        },
                        bytes: bmp.data.clone(),
                    });
                    survivors.insert(key, cached);
                }
            }
        }
        self.cache = survivors;
    }

    /// The slot of a cached glyph, stamped as drawn this frame.
    pub(super) fn lookup(&mut self, key: &GlyphKey) -> Option<AtlasSlot> {
        let cached = self.cache.get_mut(key)?;
        cached.last_used = self.frame;
        Some(cached.slot)
    }

    /// Cache a freshly rasterized glyph and queue its upload. Answers `None`
    /// when it cannot be drawn this frame. Either the colour atlas is not
    /// grown yet, or the atlas is full and compacts at the next frame.
    pub(super) fn insert(&mut self, key: GlyphKey, raster: Raster) -> Option<AtlasSlot> {
        let Raster {
            size,
            left,
            top,
            atlas,
            data,
        } = raster;
        let is_color = atlas == AtlasKind::Color;
        // Invisible glyph (for example space): cache a zero-size slot, no upload.
        if size.width == Px::ZERO || size.height == Px::ZERO || data.is_empty() {
            let slot = AtlasSlot {
                uv: UvBox::default(),
                size: Size::default(),
                bearing_left: left,
                bearing_top: top,
                atlas,
                texel_scale: 1.0,
            };
            self.cache.insert(
                key,
                CachedGlyph {
                    slot,
                    last_used: self.frame,
                    bitmap: None,
                },
            );
            return Some(slot);
        }

        // First color glyph ever: the color atlas is still a 1x1 placeholder.
        // Latch the request and drop the glyph for this frame; the renderer grows
        // the texture before the next frame, when this re-rasterizes and packs.
        if is_color && !self.color_ready {
            self.color_wanted = true;
            return None;
        }

        // Pack into the matching atlas. On overflow, flag a compaction for the
        // next frame, deferred to avoid rewriting UVs already baked into this
        // frame. The glyph is dropped for now, as the previous path did.
        let (packer, pending) = if is_color {
            (&mut self.color_packer, &mut self.color_pending)
        } else {
            (&mut self.packer, &mut self.pending)
        };
        let Some(at) = packer.allocate(size) else {
            self.needs_compact = true;
            return None;
        };

        let slot = AtlasSlot {
            uv: UvBox::of(at, size, self.size),
            size,
            bearing_left: left,
            bearing_top: top,
            atlas,
            texel_scale: 1.0,
        };

        pending.push(Upload {
            rect: TexelRect {
                x: at.x,
                y: at.y,
                size,
            },
            bytes: data.clone(),
        });
        self.cache.insert(
            key,
            CachedGlyph {
                slot,
                last_used: self.frame,
                bitmap: Some(GlyphBitmap { size, data }),
            },
        );
        Some(slot)
    }

    /// The uploads queued since the last call: the mask atlas's, then the
    /// colour atlas's.
    pub(super) fn take_uploads(&mut self) -> (Vec<Upload>, Vec<Upload>) {
        (
            std::mem::take(&mut self.pending),
            std::mem::take(&mut self.color_pending),
        )
    }
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE.get(),
                height: ATLAS_SIZE.get(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());

        // Lazy: a 1x1 placeholder until the first color glyph (see `color_ready`).
        let color_texture = Self::new_color_texture(device, Px::new(1));
        let color_view = color_texture.create_view(&Default::default());

        Self {
            texture,
            view,
            color_texture,
            color_view,
            glyphs: GlyphCache::new(ATLAS_SIZE),
            scale_cx: swash::scale::ScaleContext::new(),
        }
    }

    /// Create an `Rgba8Unorm` color-atlas texture of `size` x `size`.
    fn new_color_texture(device: &wgpu::Device, size: Px<Texel>) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_color_atlas"),
            size: wgpu::Extent3d {
                width: size.get(),
                height: size.get(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Grow the color atlas to its full size the first time a color glyph is
    /// wanted. Returns `true` when it (re)allocated the texture, so the renderer
    /// knows to rebuild the color bind group over the new `color_view`. A no-op
    /// (returning `false`) once allocated or while no color glyph has appeared.
    pub fn ensure_color_atlas(&mut self, device: &wgpu::Device) -> bool {
        if !self.glyphs.color_atlas_due() {
            return false;
        }
        self.color_texture = Self::new_color_texture(device, ATLAS_SIZE);
        self.color_view = self.color_texture.create_view(&Default::default());
        self.glyphs.color_ready = true;
        true
    }

    /// Called at the start of each frame. Advances the frame clock used to age
    /// glyphs for LRU compaction. It also runs a deferred compaction if the
    /// atlas overflowed on the previous frame. Deferring it means the UVs of
    /// glyphs already emitted into that frame's vertex buffer are never
    /// rewritten.
    pub fn begin_frame(&mut self) {
        self.glyphs.begin_frame();
    }

    /// Look up a glyph in the cache. If absent, rasterize it via Swash and
    /// queue the bitmap for upload. Returns `None` for invisible glyphs (for example
    /// whitespace) or if the atlas just became full (will clear next frame).
    /// A size that is not finite also gives `None`, and one past the raster
    /// cap is made at the cap and drawn scaled up (`CANVAS-087`).
    pub fn get_or_rasterize(
        &mut self,
        font_data: &parley::FontData,
        font_size: Px<GlyphRaster>,
        normalized_coords: &[swash::NormalizedCoord],
        glyph: GlyphId,
    ) -> Option<AtlasSlot> {
        glyph_slot(
            &mut self.glyphs,
            &mut self.scale_cx,
            FontId::of_blob(font_data.data.id()),
            || swash::FontRef::from_index(font_data.data.as_ref(), font_data.index as usize),
            font_size,
            normalized_coords,
            glyph,
        )
    }

    /// Upload all pending rasterized glyphs to the two GPU textures.
    pub fn flush_uploads(&mut self, queue: &wgpu::Queue) {
        let (pending, color_pending) = self.glyphs.take_uploads();
        for upload in pending {
            upload_glyph(queue, &self.texture, upload, AtlasKind::Mask);
        }
        for upload in color_pending {
            upload_glyph(queue, &self.color_texture, upload, AtlasKind::Color);
        }
    }
}

/// The slot of `glyph` at `size`, from the cache or rasterized into it, the
/// body of [`GlyphAtlas::get_or_rasterize`]. `font` is parsed on a miss only.
///
/// A size past [`RASTER_CAP`] is keyed and rasterized at the cap, and its slot
/// carries the ratio to draw it at (`CANVAS-087`). The bitmap stays bounded,
/// and the text stays visible.
fn glyph_slot<'a>(
    glyphs: &mut GlyphCache,
    scale_cx: &mut swash::scale::ScaleContext,
    font_id: FontId,
    font: impl FnOnce() -> Option<swash::FontRef<'a>>,
    size: Px<GlyphRaster>,
    normalized_coords: &[swash::NormalizedCoord],
    glyph: GlyphId,
) -> Option<AtlasSlot> {
    if !size.get().is_finite() {
        return None;
    }
    let drawn = capped(size);
    let key = GlyphKey::new(font_id, glyph, drawn);
    let slot = match glyphs.lookup(&key) {
        Some(slot) => slot,
        None => {
            let raster = rasterize(scale_cx, font()?, drawn, normalized_coords, glyph)?;
            glyphs.insert(key, raster)?
        }
    };
    Some(AtlasSlot {
        texel_scale: texel_scale(size),
        ..slot
    })
}

/// Rasterize one glyph at `size`, in device pixels, through Swash.
///
/// The caller passes the logical size times the density (`CANVAS-033`). A
/// colour glyph comes back as straight-alpha RGBA, anything else as an R8
/// coverage mask.
///
/// A size past [`RASTER_CAP`], or one that is not finite, gives `None` before
/// Swash runs (`CANVAS-087`). Swash allocates a mask of about the size
/// squared, so [`glyph_slot`] passes the capped size.
pub(super) fn rasterize(
    scale_cx: &mut swash::scale::ScaleContext,
    font: swash::FontRef<'_>,
    size: Px<GlyphRaster>,
    normalized_coords: &[swash::NormalizedCoord],
    glyph: GlyphId,
) -> Option<Raster> {
    if !size.get().is_finite() || size.get() > RASTER_CAP.get() {
        return None;
    }
    // Source order: an embedded colour bitmap (CBDT emoji), then a colour
    // outline (COLR), then the plain outline. `Format::Alpha` only governs how
    // outlines are rasterized; it never flattens a colour bitmap to a mask.
    let image = swash::scale::Render::new(&[
        swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
        swash::scale::Source::ColorOutline(0),
        swash::scale::Source::Outline,
    ])
    .format(swash::zeno::Format::Alpha)
    .render(
        &mut scale_cx
            .builder(font)
            .size(size.get())
            // The hinter puts the cap height, the baseline and each bar on a
            // pixel row, as the browser draws its own text (`CANVAS-099`).
            .hint(true)
            .normalized_coords(normalized_coords)
            .build(),
        glyph.get(),
    )?;
    Some(Raster {
        size: Size::new(
            Px::new(image.placement.width),
            Px::new(image.placement.height),
        ),
        left: Px::new(image.placement.left),
        top: Px::new(image.placement.top),
        atlas: match image.content {
            swash::scale::image::Content::Color => AtlasKind::Color,
            _ => AtlasKind::Mask,
        },
        data: image.data,
    })
}

/// Upload one rasterized glyph bitmap to `texture`, over its texel rectangle.
///
/// `atlas` names the format: an R8 coverage mask, one byte a texel, or RGBA
/// colour, four. WebGL2 (via the wgpu GL
/// backend) requires `bytes_per_row` to be a multiple of 4 for
/// `texSubImage2D`. The R8 path pads each row up to that alignment. The RGBA
/// path is already a multiple of 4 (`w * 4`), so it uploads as-is.
fn upload_glyph(queue: &wgpu::Queue, texture: &wgpu::Texture, upload: Upload, atlas: AtlasKind) {
    let Upload {
        rect: TexelRect { x, y, size },
        bytes: data,
    } = upload;
    let (x, y, w, h) = (x.get(), y.get(), size.width.get(), size.height.get());
    let bpp: u32 = match atlas {
        AtlasKind::Mask => 1,
        AtlasKind::Color => 4,
    };
    let row_bytes = w * bpp;
    let aligned_bpr = (row_bytes + 3) & !3;
    let upload_data: Vec<u8> = if aligned_bpr == row_bytes {
        // Already aligned -- use data as-is without copying.
        data
    } else {
        let mut padded = Vec::with_capacity((aligned_bpr * h) as usize);
        for row in data.chunks_exact(row_bytes as usize) {
            padded.extend_from_slice(row);
            padded.resize(padded.len() + (aligned_bpr - row_bytes) as usize, 0);
        }
        padded
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &upload_data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned_bpr),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The upload of `bytes` into a `w` by `h` box at `(x, y)`.
    fn upload(x: u32, y: u32, w: u32, h: u32, bytes: Vec<u8>) -> Upload {
        Upload {
            rect: TexelRect {
                x: Px::new(x),
                y: Px::new(y),
                size: Size::new(Px::new(w), Px::new(h)),
            },
            bytes,
        }
    }

    /// A square mask glyph of side `side`, filled with `fill`.
    fn mask(side: u32, fill: u8) -> Raster {
        Raster {
            size: Size::new(Px::new(side), Px::new(side)),
            left: Px::new(1),
            top: Px::new(2),
            atlas: AtlasKind::Mask,
            data: vec![fill; (side * side) as usize],
        }
    }

    fn key(glyph: u16) -> GlyphKey {
        GlyphKey::new(
            FontId::of_blob(7),
            GlyphId::new(glyph),
            Px::<GlyphRaster>::new(16.0),
        )
    }

    /// The pixel origin of a slot in an atlas of side `size`.
    fn origin(slot: AtlasSlot, size: u32) -> (u32, u32) {
        let at = |uv: f32| (uv * size as f32).round() as u32;
        (at(slot.uv.min.x.get()), at(slot.uv.min.y.get()))
    }

    /// Rasterize the glyph `ch` of a shipped face at `size` device pixels.
    /// A hinted outline puts its horizontal edges on pixel rows
    /// (`CANVAS-099`). The H of the body face has a flat top and a flat foot.
    /// The first and the last row of each stem are then as dark as its middle.
    /// Unhinted at 11 pt, the cap height falls inside a row, and the top row
    /// of each stem is half as dark.
    #[test]
    fn a_hinted_glyph_puts_its_horizontal_edges_on_pixel_rows() {
        // 11 pt at densities 1, 1.25 and 2, in device pixels.
        for size in [14.667, 18.333, 29.333] {
            let h = raster_of("NotoSans-Regular.ttf", 'H', size);
            let (w, rows) = (h.size.width.get() as usize, h.size.height.get() as usize);
            let at = |x: usize, y: usize| u32::from(h.data[y * w + x]);
            let middle = rows / 4;
            let stem = (0..w).max_by_key(|&x| at(x, middle)).expect("a stem");
            let solid = at(stem, middle);
            for (row, name) in [(0, "top"), (rows - 1, "foot")] {
                assert!(
                    at(stem, row) * 10 >= solid * 9,
                    "{name} row {} against {solid} at {size}",
                    at(stem, row)
                );
            }
        }
    }

    fn raster_of(file: &str, ch: char, size: f32) -> Raster {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        let bytes = std::fs::read(format!("{dir}{file}")).expect("a shipped face");
        let font = swash::FontRef::from_index(&bytes, 0).expect("a parsable face");
        let glyph = GlyphId::new(font.charmap().map(ch));
        let mut scale_cx = swash::scale::ScaleContext::new();
        rasterize(
            &mut scale_cx,
            font,
            Px::<GlyphRaster>::new(size),
            &[],
            glyph,
        )
        .expect("a raster")
    }

    /// A glyph rasterized for a density of 2 has twice the texels of the
    /// logical one (`CANVAS-033`).
    ///
    /// A raster at the logical size, stretched over a dense screen, would
    /// draw blurred.
    #[test]
    fn a_dense_raster_has_the_physical_resolution() {
        let logical = raster_of("NotoSans-Regular.ttf", 'H', 14.0);
        let dense = raster_of("NotoSans-Regular.ttf", 'H', 28.0);
        let (lw, lh) = (logical.size.width.get(), logical.size.height.get());
        let (dw, dh) = (dense.size.width.get(), dense.size.height.get());
        assert!(lh > 0 && logical.atlas == AtlasKind::Mask);
        assert!(dh.abs_diff(2 * lh) <= 1, "{dh} vs {lh}");
        assert!(dw.abs_diff(2 * lw) <= 1, "{dw} vs {lw}");
    }

    /// A glyph past the raster cap is rasterized at the cap, and its slot
    /// carries the ratio to draw it at. Swash never allocates a mask of the
    /// size asked, and a size that is not finite gives no glyph (`CANVAS-087`).
    #[test]
    fn a_glyph_past_the_raster_cap_is_made_at_the_cap_and_drawn_scaled() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        let bytes = std::fs::read(format!("{dir}NotoSans-Regular.ttf")).expect("a shipped face");
        let font = swash::FontRef::from_index(&bytes, 0).expect("a parsable face");
        let glyph = GlyphId::new(font.charmap().map('H'));
        let mut scale_cx = swash::scale::ScaleContext::new();
        let mut cache = GlyphCache::new(ATLAS_SIZE);
        cache.begin_frame();
        let mut slot_at = |size: f32| {
            glyph_slot(
                &mut cache,
                &mut scale_cx,
                FontId::of_blob(7),
                || Some(font),
                Px::<GlyphRaster>::new(size),
                &[],
                glyph,
            )
        };
        let at_cap = slot_at(RASTER_CAP.get()).expect("a glyph at the cap");
        assert_eq!(at_cap.texel_scale, 1.0);
        for size in [ATLAS_SIZE.get() as f32 + 1.0, 1.0e6] {
            let past = slot_at(size).expect("a glyph past the cap");
            assert_eq!(past.size, at_cap.size, "{size}");
            assert_eq!(past.uv.min, at_cap.uv.min, "one bitmap, {size}");
            assert_eq!(past.texel_scale, size / RASTER_CAP.get(), "{size}");
        }
        assert!(slot_at(f32::INFINITY).is_none());
        assert!(slot_at(f32::NAN).is_none());
        let past = Px::<GlyphRaster>::new(RASTER_CAP.get() + 1.0);
        assert!(rasterize(&mut scale_cx, font, past, &[], glyph).is_none());
    }

    /// An emoji rasterizes to colour and a letter to a mask, which is what
    /// routes each to its own atlas (`CANVAS-019`).
    #[test]
    fn an_emoji_rasterizes_to_colour_and_a_letter_to_a_mask() {
        let emoji = raster_of("NotoColorEmoji.ttf", '\u{1F44D}', 28.0);
        let texels = |r: &Raster| (r.size.width.get() * r.size.height.get()) as usize;
        assert!(emoji.atlas == AtlasKind::Color && emoji.size.height.get() > 0);
        assert_eq!(emoji.data.len(), texels(&emoji) * 4, "RGBA");
        let letter = raster_of("NotoSans-Regular.ttf", 'a', 28.0);
        assert_eq!(letter.data.len(), texels(&letter), "R8");
    }

    /// A glyph goes right of the last one, with one pixel of padding. When
    /// the row is full, a shelf opens below the row's tallest glyph. A glyph
    /// taller than what is left does not fit.
    #[test]
    fn a_shelf_opens_below_when_a_row_is_full() {
        let mut packer = ShelfPacker::new(Px::new(10));
        let mut allocate = |w: u32, h: u32| {
            packer
                .allocate(Size::new(Px::new(w), Px::new(h)))
                .map(|at| (at.x.get(), at.y.get()))
        };
        assert_eq!(allocate(4, 3), Some((0, 0)));
        assert_eq!(allocate(4, 2), Some((5, 0)));
        assert_eq!(allocate(4, 5), Some((0, 4)));
        assert_eq!(allocate(3, 7), None);
        packer.reset();
        assert_eq!(
            packer.allocate(Size::new(Px::new(4), Px::new(3))),
            Some(Point::new(Px::ZERO, Px::ZERO))
        );
    }

    /// A packed glyph is found again with the slot it was given. Its bitmap
    /// is queued once, and the queue empties when the uploads are taken.
    #[test]
    fn a_packed_glyph_is_found_and_uploaded_once() {
        let mut cache = GlyphCache::new(Px::new(64));
        cache.begin_frame();
        let slot = cache.insert(key(1), mask(4, 9)).expect("the glyph fits");
        assert_eq!(slot.size, Size::new(Px::new(4), Px::new(4)));
        assert_eq!(
            (slot.bearing_left, slot.bearing_top),
            (Px::new(1), Px::new(2))
        );
        let quarter = Px::<Uv>::new(4.0 / 64.0);
        assert_eq!(slot.uv.max, Point::new(quarter, quarter));
        let again = cache.lookup(&key(1)).expect("the glyph is cached");
        assert_eq!(again.uv, slot.uv);
        assert!(cache.lookup(&key(2)).is_none());
        let (masks, colors) = cache.take_uploads();
        assert_eq!(masks, vec![upload(0, 0, 4, 4, vec![9; 16])]);
        assert!(colors.is_empty());
        assert_eq!(cache.take_uploads(), (Vec::new(), Vec::new()));
    }

    /// A full atlas drops the glyph for this frame and moves nothing. The
    /// next frame repacks the glyphs drawn lately, the most recent first, and
    /// drops the ones not drawn for more than two frames (`CANVAS-021`).
    #[test]
    fn an_overflow_repacks_the_hot_glyphs_at_the_next_frame() {
        let mut cache = GlyphCache::new(Px::new(16));
        cache.begin_frame();
        cache.insert(key(1), mask(4, 1)).expect("cold fits");
        for _ in 0..3 {
            cache.begin_frame();
        }
        cache.insert(key(2), mask(4, 2)).expect("warm fits");
        cache.begin_frame();
        let hot = cache.insert(key(3), mask(4, 3)).expect("hot fits");
        cache.take_uploads();

        assert!(
            cache.insert(key(4), mask(16, 4)).is_none(),
            "the atlas is full"
        );
        // Drawn again this frame: the hot glyph keeps its place until the next.
        let unmoved = cache.lookup(&key(3)).expect("still cached");
        assert_eq!(unmoved.uv.min, hot.uv.min, "nothing moves mid-frame");
        assert_eq!(cache.take_uploads(), (Vec::new(), Vec::new()));

        cache.begin_frame();
        assert!(cache.lookup(&key(1)).is_none(), "the cold glyph is dropped");
        let hot = cache.lookup(&key(3)).expect("the hot glyph survives");
        let warm = cache.lookup(&key(2)).expect("the warm glyph survives");
        assert_eq!(origin(hot, 16), (0, 0), "the most recent goes first");
        assert_eq!(origin(warm, 16), (5, 0));
        let (masks, _) = cache.take_uploads();
        let bytes: Vec<u8> = masks.iter().map(|upload| upload.bytes[0]).collect();
        assert_eq!(bytes, [3, 2], "both are uploaded again, from their bitmaps");
        assert_eq!(cache.size, Px::new(16), "the atlas repacks and never grows");
    }

    /// Covers `CANVAS-054`: a glyph that survives a repack draws what it drew.
    /// Its bitmap, size and bearings are the ones it was rasterized with.
    #[test]
    fn a_repack_moves_a_surviving_glyph_and_changes_nothing_it_draws() {
        let mut cache = GlyphCache::new(Px::new(16));
        cache.begin_frame();
        cache.insert(key(1), mask(4, 1)).expect("cold fits");
        cache.begin_frame();
        cache.begin_frame();
        cache.begin_frame();
        let mut glyph = mask(5, 0);
        glyph.data = (0..25).collect();
        let before = cache.insert(key(2), glyph).expect("hot fits");
        cache.take_uploads();
        assert!(
            cache.insert(key(3), mask(16, 3)).is_none(),
            "the atlas is full"
        );
        cache.begin_frame();
        let after = cache.lookup(&key(2)).expect("the hot glyph survives");
        assert_ne!(after.uv.min, before.uv.min, "the repack moved it");
        let size = |s: AtlasSlot| (s.size, s.bearing_left, s.bearing_top);
        assert_eq!(size(after), size(before));
        let (masks, _) = cache.take_uploads();
        let (x, y) = origin(after, 16);
        let expected: Vec<u8> = (0..25).collect();
        assert_eq!(
            masks,
            vec![upload(x, y, 5, 5, expected)],
            "its own bitmap, at its new place"
        );
    }

    /// Covers `CANVAS-020`: a session drawing no colour glyph never asks for
    /// the full colour atlas. The first colour glyph asks for it once.
    #[test]
    fn the_color_atlas_is_grown_only_for_a_color_glyph() {
        let mut cache = GlyphCache::new(Px::new(64));
        for glyph in 0..20 {
            cache.begin_frame();
            cache.insert(key(glyph), mask(4, 1));
        }
        assert!(!cache.color_atlas_due(), "no colour glyph, no colour atlas");
        let emoji = Raster {
            atlas: AtlasKind::Color,
            data: vec![5; 4 * 4 * 4],
            ..mask(4, 0)
        };
        assert!(cache.insert(key(99), emoji).is_none());
        assert!(
            cache.color_atlas_due(),
            "the first colour glyph asks for it"
        );
        cache.color_ready = true;
        assert!(!cache.color_atlas_due(), "grown once");
    }

    /// An invisible glyph takes no space and queues nothing, and a
    /// compaction keeps it.
    #[test]
    fn an_invisible_glyph_takes_no_space() {
        let mut cache = GlyphCache::new(Px::new(8));
        cache.begin_frame();
        let blank = Raster {
            data: Vec::new(),
            ..mask(0, 0)
        };
        let slot = cache.insert(key(1), blank).expect("a blank slot");
        assert_eq!((slot.size, slot.uv), (Size::default(), UvBox::default()));
        assert_eq!(cache.take_uploads(), (Vec::new(), Vec::new()));
        assert!(
            cache.insert(key(2), mask(8, 1)).is_none(),
            "the atlas is full"
        );
        cache.begin_frame();
        assert!(cache.lookup(&key(1)).is_some(), "the blank slot survives");
    }

    /// A colour glyph seen before the colour atlas is grown is dropped and
    /// asks for it. Once grown, it packs into the colour atlas alone.
    #[test]
    fn a_color_glyph_waits_for_the_color_atlas() {
        let mut cache = GlyphCache::new(Px::new(32));
        cache.begin_frame();
        let emoji = || Raster {
            atlas: AtlasKind::Color,
            data: vec![5; 4 * 4 * 4],
            ..mask(4, 0)
        };
        assert!(cache.insert(key(1), emoji()).is_none());
        assert!(cache.color_wanted, "the first sighting asks for the atlas");
        cache.color_ready = true;
        let slot = cache.insert(key(1), emoji()).expect("the atlas is grown");
        assert_eq!(slot.atlas, AtlasKind::Color);
        assert_eq!(origin(slot, 32), (0, 0));
        let mask_slot = cache.insert(key(2), mask(4, 1)).expect("a mask fits");
        assert_eq!(origin(mask_slot, 32), (0, 0), "each atlas packs alone");
        let (masks, colors) = cache.take_uploads();
        assert_eq!((masks.len(), colors.len()), (1, 1));
        assert_eq!(colors[0].bytes.len(), 64, "four bytes a pixel");
    }
}
