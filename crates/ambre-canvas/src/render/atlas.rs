// ambre-canvas/src/render/atlas.rs
// ============================================================================
// GlyphAtlas: Swash rasterization into a 2048x2048 R8Unorm GPU texture, a shelf
// packer, and the per-glyph UV/bearing cache. Batched CPU->GPU uploads once per
// frame.
// ============================================================================

use rustc_hash::FxHashMap;

// ============================================================================
// GlyphAtlas
// ============================================================================

const ATLAS_SIZE: u32 = 2048;
// 1-pixel padding between glyphs prevents bilinear bleed at atlas boundaries.
const ATLAS_PAD: u32 = 1;

#[derive(Clone, PartialEq, Eq, Hash)]
struct GlyphKey {
    font_id: u64, // Blob::id() -- stable identifier per registered font
    glyph_id: u16,
    size_x100: u32, // font_size * 100 to avoid floats in hash key
}

/// Retained alpha bitmap for a packed glyph, kept so the atlas can repack its
/// working set on overflow (LRU compaction) without re-rasterizing through
/// Swash. Invisible glyphs (space) have no bitmap.
struct GlyphBitmap {
    w: u32,
    h: u32,
    data: Vec<u8>,
}

/// A cache entry: its current atlas slot, the frame it was last drawn on (for
/// LRU compaction), and the retained bitmap (`None` for a zero-size glyph).
struct CachedGlyph {
    slot: AtlasSlot,
    last_used: u64,
    bitmap: Option<GlyphBitmap>,
}

/// UV rect and pixel-space bearing for one atlas entry.
#[derive(Clone, Copy)]
pub struct AtlasSlot {
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Horizontal offset from the glyph origin to the bitmap left edge (px).
    pub bearing_left: i32,
    /// Vertical offset from the glyph origin (baseline) to the bitmap top (px).
    /// Positive = above baseline. Apply as: `screen_y = origin_y - bearing_top`.
    pub bearing_top: i32,
}

struct ShelfPacker {
    cur_x: u32,
    cur_y: u32,   // top of the current shelf
    shelf_h: u32, // max glyph height on the current shelf
    atlas_size: u32,
}

impl ShelfPacker {
    fn new(atlas_size: u32) -> Self {
        Self {
            cur_x: 0,
            cur_y: 0,
            shelf_h: 0,
            atlas_size,
        }
    }

    fn reset(&mut self) {
        self.cur_x = 0;
        self.cur_y = 0;
        self.shelf_h = 0;
    }

    /// Try to allocate a region of `(w, h)` pixels.
    /// Returns the top-left pixel coordinate on success, or `None` if the
    /// atlas is full (caller should reset the atlas and retry).
    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let w_padded = w + ATLAS_PAD;
        // Advance to next shelf if this glyph doesn't fit horizontally.
        if self.cur_x + w_padded > self.atlas_size {
            self.cur_y += self.shelf_h + ATLAS_PAD;
            self.cur_x = 0;
            self.shelf_h = 0;
        }
        // Check vertical overflow.
        if self.cur_y + h > self.atlas_size {
            return None;
        }
        let (x, y) = (self.cur_x, self.cur_y);
        self.cur_x += w_padded;
        self.shelf_h = self.shelf_h.max(h);
        Some((x, y))
    }
}

/// CPU + GPU glyph atlas.
///
/// Glyphs are rasterized via Swash into a 2048x2048 R8Unorm GPU texture.
/// Pending CPU-to-GPU uploads are batched and flushed once per frame.
pub struct GlyphAtlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    cache: FxHashMap<GlyphKey, CachedGlyph>,
    packer: ShelfPacker,
    scale_cx: swash::scale::ScaleContext,
    /// Queued texture uploads: (x, y, w, h, alpha_data).
    pending: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    /// Monotonic frame counter, bumped in `begin_frame`. A glyph drawn this
    /// frame has `last_used == frame`; compaction keeps that working set.
    frame: u64,
    /// Set when an allocation overflowed mid-frame. Compaction is deferred to
    /// the next `begin_frame` so it never moves a glyph whose UVs were already
    /// baked into this frame's vertex buffer.
    needs_compact: bool,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
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

        Self {
            texture,
            view,
            cache: FxHashMap::default(),
            packer: ShelfPacker::new(ATLAS_SIZE),
            scale_cx: swash::scale::ScaleContext::new(),
            pending: Vec::new(),
            frame: 0,
            needs_compact: false,
        }
    }

    /// Called at the start of each frame. Advances the frame clock used to age
    /// glyphs for LRU compaction, and runs a deferred compaction if the atlas
    /// overflowed on the previous frame (deferred so it never rewrites the UVs
    /// of glyphs already emitted into that frame's vertex buffer).
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        if self.needs_compact {
            self.compact();
            self.needs_compact = false;
        }
    }

    /// Repack the recently-used glyphs into a fresh atlas, dropping the cold
    /// ones. Called at frame start after an overflow. Instead of the old
    /// full-clear (which threw away every glyph, including the ones still on
    /// screen, causing a blank flash), we keep glyphs touched within the last
    /// `KEEP_FRAMES` frames, re-pack them from their retained bitmaps (no
    /// re-rasterization), and re-queue those bitmaps for upload.
    fn compact(&mut self) {
        // Keep the current frame plus a little history so glyphs that flicker
        // in and out across a couple of frames are not thrashed.
        const KEEP_FRAMES: u64 = 2;
        let cutoff = self.frame.saturating_sub(KEEP_FRAMES);

        self.packer.reset();
        self.pending.clear();

        let mut survivors: FxHashMap<GlyphKey, CachedGlyph> = FxHashMap::default();
        // Re-pack in descending recency so the hottest glyphs are placed first
        // and are the least likely to be evicted if the working set is tight.
        let mut entries: Vec<(GlyphKey, CachedGlyph)> = self
            .cache
            .drain()
            .filter(|(_, c)| c.last_used > cutoff)
            .collect();
        entries.sort_by_key(|(_, c)| std::cmp::Reverse(c.last_used));

        let inv = 1.0 / ATLAS_SIZE as f32;
        for (key, mut cached) in entries {
            match &cached.bitmap {
                // Zero-size (invisible) glyph: no atlas space, keep as-is.
                None => {
                    survivors.insert(key, cached);
                }
                Some(bmp) => {
                    let Some((ax, ay)) = self.packer.allocate(bmp.w, bmp.h) else {
                        // Working set itself overflows: drop the rest.
                        continue;
                    };
                    cached.slot.uv_min = [ax as f32 * inv, ay as f32 * inv];
                    cached.slot.uv_max = [(ax + bmp.w) as f32 * inv, (ay + bmp.h) as f32 * inv];
                    self.pending.push((ax, ay, bmp.w, bmp.h, bmp.data.clone()));
                    survivors.insert(key, cached);
                }
            }
        }
        self.cache = survivors;
    }

    /// Look up a glyph in the cache. If absent, rasterize it via Swash and
    /// queue the bitmap for upload. Returns `None` for invisible glyphs (e.g.
    /// whitespace) or if the atlas just became full (will clear next frame).
    pub fn get_or_rasterize(
        &mut self,
        font_data: &parley::FontData,
        font_size: f32,
        normalized_coords: &[swash::NormalizedCoord],
        glyph_id: u16,
    ) -> Option<AtlasSlot> {
        let key = GlyphKey {
            font_id: font_data.data.id(),
            glyph_id,
            size_x100: (font_size * 100.0) as u32,
        };

        if let Some(cached) = self.cache.get_mut(&key) {
            cached.last_used = self.frame;
            return Some(cached.slot);
        }

        // Rasterize via Swash.
        let font_ref =
            swash::FontRef::from_index(font_data.data.as_ref(), font_data.index as usize)?;

        let image = swash::scale::Render::new(&[
            swash::scale::Source::ColorOutline(0),
            swash::scale::Source::Outline,
        ])
        .format(swash::zeno::Format::Alpha)
        .render(
            &mut self
                .scale_cx
                .builder(font_ref)
                .size(font_size)
                .normalized_coords(normalized_coords)
                .build(),
            glyph_id,
        )?;

        let w = image.placement.width;
        let h = image.placement.height;

        // Invisible glyph (e.g. space): cache a zero-size slot, no upload.
        if w == 0 || h == 0 || image.data.is_empty() {
            let slot = AtlasSlot {
                uv_min: [0.0; 2],
                uv_max: [0.0; 2],
                pixel_width: 0,
                pixel_height: 0,
                bearing_left: image.placement.left,
                bearing_top: image.placement.top,
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

        // Pack into atlas. On overflow, flag a compaction for the next frame
        // (deferred to avoid rewriting UVs already baked into this frame) and
        // drop the glyph for now, exactly as the previous full-clear path did.
        let (ax, ay) = match self.packer.allocate(w, h) {
            Some(pos) => pos,
            None => {
                self.needs_compact = true;
                return None;
            }
        };

        let inv = 1.0 / ATLAS_SIZE as f32;
        let slot = AtlasSlot {
            uv_min: [ax as f32 * inv, ay as f32 * inv],
            uv_max: [(ax + w) as f32 * inv, (ay + h) as f32 * inv],
            pixel_width: w,
            pixel_height: h,
            bearing_left: image.placement.left,
            bearing_top: image.placement.top,
        };

        self.pending.push((ax, ay, w, h, image.data.clone()));
        self.cache.insert(
            key,
            CachedGlyph {
                slot,
                last_used: self.frame,
                bitmap: Some(GlyphBitmap {
                    w,
                    h,
                    data: image.data,
                }),
            },
        );
        Some(slot)
    }

    /// Upload all pending rasterized glyphs to the GPU texture.
    pub fn flush_uploads(&mut self, queue: &wgpu::Queue) {
        for (x, y, w, h, data) in self.pending.drain(..) {
            // WebGL2 (via wgpu GL backend) requires bytes_per_row to be a
            // multiple of 4 for texSubImage2D. Pad each row to alignment.
            // Atlas is R8Unorm so bytes_per_pixel = 1.
            let aligned_bpr = (w + 3) & !3;
            let upload_data: Vec<u8> = if aligned_bpr == w {
                // Already aligned -- use data as-is without copying.
                data
            } else {
                let mut padded = Vec::with_capacity((aligned_bpr * h) as usize);
                for row in data.chunks_exact(w as usize) {
                    padded.extend_from_slice(row);
                    padded.resize(padded.len() + (aligned_bpr - w) as usize, 0);
                }
                padded
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
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
    }
}
