// ambre-canvas/src/render/mod.rs
// ============================================================================
// CPU-batched renderer backed by wgpu vertex/fragment shaders.
//
// No compute shaders -- every draw call maps to plain triangles + textured quads.
//
// Three pipelines, flushed in order each frame:
//   1. Quad pipeline  -- solid-color filled/stroked rectangles and lines.
//   2. Glyph pipeline -- alpha-mask glyphs from the GlyphAtlas (R8Unorm).
//   3. Blit pipeline  -- samples the persistent cells offscreen onto swapchain.
//
// Dirty-rect rendering:
//   The persistent CELLS layer (scrollable cell area + grid lines + cell text)
//   is rendered to a ping-pong pair of offscreen textures. Each frame can
//   either repaint everything (Full), shift the previous frame via
//   `copy_texture_to_texture` and redraw only the entering strip (Scroll), or
//   skip the cells layer entirely (None) and just re-blit. The OVERLAY layer
//   (headers, active border, HUD) is transient and re-rendered to the
//   swapchain every dirty frame.
//
// Public API:
//   QuadRenderer  -- owns all pipelines, GlyphAtlas, per-layer CPU batches and
//                    GPU buffers, plus the ping-pong offscreen pair.
//   WgpuSurface   -- frame-scoped DrawSurface. Holds the clip stack.
//                    Routes pushes to the renderer's currently active layer.
// ============================================================================

use ambre_draw::{Affine, Brush, Color, DrawSurface, GlyphRun, ParleyBrush, Point, Rect};
use bytemuck::{Pod, Zeroable};
use rustc_hash::FxHashMap;

// ============================================================================
// Quad pipeline
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadVertex {
    pos: [f32; 2],
    color: [f32; 4],
}

const QUAD_VERTEX_SIZE: usize = std::mem::size_of::<QuadVertex>();
const INITIAL_QUADS: usize = 8192;

// ============================================================================
// Glyph pipeline
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlyphVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

const GLYPH_VERTEX_SIZE: usize = std::mem::size_of::<GlyphVertex>();
const INITIAL_GLYPHS: usize = 4096;

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
    cache: FxHashMap<GlyphKey, AtlasSlot>,
    packer: ShelfPacker,
    scale_cx: swash::scale::ScaleContext,
    /// Queued texture uploads: (x, y, w, h, alpha_data).
    pending: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    needs_clear: bool,
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
            needs_clear: false,
        }
    }

    /// Called at the start of each frame. If the atlas overflowed last frame,
    /// clear it now so the new frame starts with a clean state.
    pub fn begin_frame(&mut self) {
        if self.needs_clear {
            self.cache.clear();
            self.packer.reset();
            self.pending.clear();
            self.needs_clear = false;
        }
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

        if let Some(&slot) = self.cache.get(&key) {
            return Some(slot);
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
            self.cache.insert(key, slot);
            return Some(slot);
        }

        // Pack into atlas.
        let (ax, ay) = match self.packer.allocate(w, h) {
            Some(pos) => pos,
            None => {
                // Atlas full -- will clear at next begin_frame().
                self.needs_clear = true;
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

        self.pending.push((ax, ay, w, h, image.data));
        self.cache.insert(key, slot);
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

// ============================================================================
// Layer batches
// ============================================================================

/// Identifies which set of batches receives subsequent `push_quad` / `push_glyph`
/// calls. The CELLS layer renders to a ping-pong offscreen texture and may be
/// reused across frames via dirty-rect updates. The OVERLAY layer renders
/// directly to the swapchain on top of the blitted cells texture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Content,
    Overlay,
}

/// Per-layer CPU batches + GPU buffers. Each layer owns its own VB/IB so the
/// two passes can be encoded into the same command encoder without overwriting
/// each other. The `drawn_*` fields record the last uploaded counts so idle
/// frames can re-encode draw calls using the previously uploaded GPU buffers
/// without rebuilding the CPU side.
struct LayerBatches {
    quad_vertices: Vec<QuadVertex>,
    quad_indices:  Vec<u32>,
    glyph_vertices: Vec<GlyphVertex>,
    glyph_indices:  Vec<u32>,
    quad_vb: wgpu::Buffer,
    quad_ib: wgpu::Buffer,
    glyph_vb: wgpu::Buffer,
    glyph_ib: wgpu::Buffer,
    /// Number of quad indices in the last upload. Used as the draw_indexed
    /// upper bound on any subsequent frame that reuses the GPU buffer.
    drawn_quad_indices:  u32,
    drawn_glyph_indices: u32,
    /// Set to `true` by `begin_rebuild()`. When set, `upload_if_rebuilding()`
    /// transfers the CPU vecs to the GPU and refreshes `drawn_*`. Otherwise
    /// the GPU buffer state is preserved across frames.
    rebuilding: bool,
}

impl LayerBatches {
    fn new(device: &wgpu::Device, label_prefix: &str) -> Self {
        let quad_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_quad_vb")),
            size: (INITIAL_QUADS * 4 * QUAD_VERTEX_SIZE) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_quad_ib")),
            size: (INITIAL_QUADS * 6 * 4) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_glyph_vb")),
            size: (INITIAL_GLYPHS * 4 * GLYPH_VERTEX_SIZE) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_glyph_ib")),
            size: (INITIAL_GLYPHS * 6 * 4) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            quad_vertices: Vec::with_capacity(INITIAL_QUADS * 4),
            quad_indices:  Vec::with_capacity(INITIAL_QUADS * 6),
            glyph_vertices: Vec::with_capacity(INITIAL_GLYPHS * 4),
            glyph_indices:  Vec::with_capacity(INITIAL_GLYPHS * 6),
            quad_vb,
            quad_ib,
            glyph_vb,
            glyph_ib,
            drawn_quad_indices:  0,
            drawn_glyph_indices: 0,
            rebuilding: false,
        }
    }

    /// Caller signals that this layer's CPU batch will be rebuilt from
    /// scratch this frame. Clears the CPU vecs and arms `upload_if_rebuilding`.
    fn begin_rebuild(&mut self) {
        self.quad_vertices.clear();
        self.quad_indices.clear();
        self.glyph_vertices.clear();
        self.glyph_indices.clear();
        self.rebuilding = true;
    }

    /// Upload the CPU batches to the GPU only if `begin_rebuild` was called
    /// this frame. Otherwise the GPU buffer state and `drawn_*` counts persist
    /// from the previous upload.
    fn upload_if_rebuilding(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label_prefix: &str,
    ) {
        if !self.rebuilding {
            return;
        }
        self.rebuilding = false;
        if !self.quad_indices.is_empty() {
            grow_buffer_if_needed(
                device,
                &mut self.quad_vb,
                self.quad_vertices.len() * QUAD_VERTEX_SIZE,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "quad_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.quad_ib,
                self.quad_indices.len() * 4,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                "quad_ib",
                label_prefix,
            );
            queue.write_buffer(&self.quad_vb, 0, bytemuck::cast_slice(&self.quad_vertices));
            queue.write_buffer(&self.quad_ib, 0, bytemuck::cast_slice(&self.quad_indices));
        }
        if !self.glyph_indices.is_empty() {
            grow_buffer_if_needed(
                device,
                &mut self.glyph_vb,
                self.glyph_vertices.len() * GLYPH_VERTEX_SIZE,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "glyph_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.glyph_ib,
                self.glyph_indices.len() * 4,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                "glyph_ib",
                label_prefix,
            );
            queue.write_buffer(
                &self.glyph_vb,
                0,
                bytemuck::cast_slice(&self.glyph_vertices),
            );
            queue.write_buffer(&self.glyph_ib, 0, bytemuck::cast_slice(&self.glyph_indices));
        }
        // Refresh the drawn counts for subsequent encode passes.
        self.drawn_quad_indices  = self.quad_indices.len()  as u32;
        self.drawn_glyph_indices = self.glyph_indices.len() as u32;
    }
}

// ============================================================================
// Offscreen target
// ============================================================================

struct OffscreenTarget {
    texture: wgpu::Texture,
    view:    wgpu::TextureView,
}

impl OffscreenTarget {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width:                   width.max(1),
                height:                  height.max(1),
                depth_or_array_layers:   1,
            },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }
}

// ============================================================================
// ContentRepaint
// ============================================================================

/// How to repaint the CONTENT (persistent) layer this frame.
///
/// `Scroll` shifts the previous frame's offscreen via `copy_texture_to_texture`
/// then renders the freshly pushed content geometry on top (typically the strip
/// entering the viewport). `Reuse` skips all content work and re-blits the
/// existing offscreen unchanged — the orchestrator must NOT have called
/// `begin_layer(Layer::Content)` in this case.
#[derive(Clone, Copy, Debug)]
pub enum ContentRepaint {
    Full,
    Scroll { dx_px: i32, dy_px: i32 },
    Reuse,
}

/// How to repaint the OVERLAY (transient, swapchain) layer this frame.
///
/// `Redraw` uploads the freshly pushed overlay geometry to the GPU.
/// `Reuse` skips the upload and re-issues the draw call using the previously
/// uploaded GPU buffer — used on idle frames where nothing visible changed.
#[derive(Clone, Copy, Debug)]
pub enum OverlayRepaint {
    Redraw,
    Reuse,
}


// ============================================================================
// QuadRenderer
// ============================================================================

/// Owns all wgpu pipelines (quad, glyph, blit), the GlyphAtlas, ping-pong
/// offscreen pair for the persistent cells layer, and per-layer CPU+GPU
/// batches.
pub struct QuadRenderer {
    // -- Pipelines --
    quad_pipeline:  wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    blit_pipeline:  wgpu::RenderPipeline,

    // -- Shared uniform (viewport size) --
    uniform_buf: wgpu::Buffer,
    uniform_bg:  wgpu::BindGroup,

    // -- Glyph atlas + its bind group --
    atlas_bind_group: wgpu::BindGroup,
    pub atlas:        GlyphAtlas,

    // -- Per-layer batches --
    cells:   LayerBatches,
    overlay: LayerBatches,
    active_layer: Layer,

    // -- Ping-pong offscreen pair for the cells layer --
    offscreen_format: wgpu::TextureFormat,
    offscreens:       [OffscreenTarget; 2],
    blit_bind_groups: [wgpu::BindGroup; 2],
    blit_bgl:         wgpu::BindGroupLayout,
    blit_sampler:     wgpu::Sampler,
    /// Index of the offscreen that holds the latest valid CELLS pixels.
    /// After a Full/Scroll repaint, we render into the OTHER index and flip.
    current_offscreen_idx: usize,

    // -- Viewport state --
    width:  f32,
    height: f32,
}

impl QuadRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        // -- Shared viewport uniform --
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad_uniforms"),
            size: 16, // vec2<f32> + 8-byte pad
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform_bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        // The cells offscreen uses the same format as the swapchain so the
        // blit pipeline can sample it without format conversion.
        let offscreen_format = surface_format;

        // -- Quad pipeline (writes to either offscreen_format or surface_format
        //    -- both are the same here, so a single pipeline works). --
        let quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("solid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("solid.wgsl").into()),
        });
        let quad_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad_pl"),
            bind_group_layouts: &[&uniform_bgl],
            immediate_size: 0,
        });
        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad"),
            layout: Some(&quad_pl_layout),
            vertex: wgpu::VertexState {
                module: &quad_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: QUAD_VERTEX_SIZE as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &quad_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // -- Glyph pipeline --
        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glyph"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glyph.wgsl").into()),
        });
        let atlas_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let glyph_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glyph_pl"),
            bind_group_layouts: &[&uniform_bgl, &atlas_bgl],
            immediate_size: 0,
        });
        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph"),
            layout: Some(&glyph_pl_layout),
            vertex: wgpu::VertexState {
                module: &glyph_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: GLYPH_VERTEX_SIZE as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &glyph_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // -- Blit pipeline (samples persistent cells offscreen onto swapchain) --
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });
        let blit_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit_pl"),
            bind_group_layouts: &[&blit_bgl],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&blit_pl_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None, // opaque overwrite -- cells layer is the base
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // -- GlyphAtlas + its bind group --
        let atlas = GlyphAtlas::new(device);
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas_bg"),
            layout: &atlas_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        // -- Blit sampler --
        // NonFiltering nearest-neighbour: the offscreen is 1:1 with the
        // swapchain so we want exact pixel copy with no interpolation.
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // -- Ping-pong offscreen pair (initial dummy 1x1, sized on first frame) --
        let offscreens = [
            OffscreenTarget::new(device, offscreen_format, 1, 1, "cells_offscreen_a"),
            OffscreenTarget::new(device, offscreen_format, 1, 1, "cells_offscreen_b"),
        ];
        let blit_bind_groups = [
            make_blit_bg(device, &blit_bgl, &offscreens[0].view, &blit_sampler, "blit_bg_a"),
            make_blit_bg(device, &blit_bgl, &offscreens[1].view, &blit_sampler, "blit_bg_b"),
        ];

        let cells   = LayerBatches::new(device, "cells");
        let overlay = LayerBatches::new(device, "overlay");

        Self {
            quad_pipeline,
            glyph_pipeline,
            blit_pipeline,
            uniform_buf,
            uniform_bg,
            atlas_bind_group,
            atlas,
            cells,
            overlay,
            active_layer: Layer::Content,
            offscreen_format,
            offscreens,
            blit_bind_groups,
            blit_bgl,
            blit_sampler,
            current_offscreen_idx: 0,
            width:  1.0,
            height: 1.0,
        }
    }

    /// Resize the ping-pong offscreens. Must be called whenever the canvas
    /// size changes. Both offscreens are reallocated and their content lost;
    /// the next frame must use `ContentRepaint::Full`.
    pub fn resize_offscreens(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.offscreens = [
            OffscreenTarget::new(device, self.offscreen_format, width, height, "cells_offscreen_a"),
            OffscreenTarget::new(device, self.offscreen_format, width, height, "cells_offscreen_b"),
        ];
        self.blit_bind_groups = [
            make_blit_bg(device, &self.blit_bgl, &self.offscreens[0].view, &self.blit_sampler, "blit_bg_a"),
            make_blit_bg(device, &self.blit_bgl, &self.offscreens[1].view, &self.blit_sampler, "blit_bg_b"),
        ];
        self.current_offscreen_idx = 0;
    }

    /// Per-frame init: ticks the atlas and updates the cached viewport size.
    /// Does NOT touch per-layer CPU batches — call `begin_layer` to clear and
    /// arm a layer for rebuild this frame; layers not begun keep their last
    /// uploaded GPU state and can be redrawn from cache (idle frames).
    pub fn begin_frame(&mut self, width: f32, height: f32) {
        self.atlas.begin_frame();
        self.width  = width;
        self.height = height;
    }

    /// Mark `layer` as being rebuilt this frame: clears its CPU batch, sets it
    /// as the active push target, and arms `upload_if_rebuilding` so the next
    /// `flush_dirty_rect` will transfer the new geometry to the GPU.
    pub fn begin_layer(&mut self, layer: Layer) {
        self.batches_mut(layer).begin_rebuild();
        self.active_layer = layer;
    }

    fn batches_mut(&mut self, layer: Layer) -> &mut LayerBatches {
        match layer {
            Layer::Content   => &mut self.cells,
            Layer::Overlay => &mut self.overlay,
        }
    }

    fn active_batches_mut(&mut self) -> &mut LayerBatches {
        let l = self.active_layer;
        self.batches_mut(l)
    }

    // -- Quad geometry --------------------------------------------------------

    pub fn push_quad(&mut self, rect: Rect, color: Color) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let c = color_f32(color);
        let x0 = rect.x0 as f32;
        let y0 = rect.y0 as f32;
        let x1 = rect.x1 as f32;
        let y1 = rect.y1 as f32;
        let b = self.active_batches_mut();
        let base = b.quad_vertices.len() as u32;
        b.quad_vertices.extend_from_slice(&[
            QuadVertex { pos: [x0, y0], color: c },
            QuadVertex { pos: [x1, y0], color: c },
            QuadVertex { pos: [x1, y1], color: c },
            QuadVertex { pos: [x0, y1], color: c },
        ]);
        b.quad_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn push_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color, width: f32) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = dx.hypot(dy);
        if len < 0.001 {
            return;
        }
        let hw = width * 0.5;
        let nx = -dy / len * hw;
        let ny = dx / len * hw;
        let c = color_f32(color);
        let b = self.active_batches_mut();
        let base = b.quad_vertices.len() as u32;
        b.quad_vertices.extend_from_slice(&[
            QuadVertex { pos: [x0 + nx, y0 + ny], color: c },
            QuadVertex { pos: [x0 - nx, y0 - ny], color: c },
            QuadVertex { pos: [x1 - nx, y1 - ny], color: c },
            QuadVertex { pos: [x1 + nx, y1 + ny], color: c },
        ]);
        b.quad_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // -- Glyph geometry -------------------------------------------------------
    #[allow(clippy::too_many_arguments)]
    pub fn push_glyph(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        uv_min: [f32; 2],
        uv_max: [f32; 2],
        color: Color,
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let c = color_f32(color);
        let b = self.active_batches_mut();
        let base = b.glyph_vertices.len() as u32;
        b.glyph_vertices.extend_from_slice(&[
            GlyphVertex { pos: [x, y],         uv: [uv_min[0], uv_min[1]], color: c },
            GlyphVertex { pos: [x + w, y],     uv: [uv_max[0], uv_min[1]], color: c },
            GlyphVertex { pos: [x + w, y + h], uv: [uv_max[0], uv_max[1]], color: c },
            GlyphVertex { pos: [x, y + h],     uv: [uv_min[0], uv_max[1]], color: c },
        ]);
        b.glyph_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    // -- Frame flush ----------------------------------------------------------

    /// Encode the three-pass dirty-rect rendering for this frame.
    ///
    /// `cells`:
    /// - `Full`   : clear the destination offscreen, render the freshly pushed
    ///   cells geometry.
    /// - `Scroll` : `copy_texture_to_texture` from the current offscreen to
    ///   the other with the requested pixel offset, then render
    ///   the cells geometry over the exposed strip.
    /// - `Reuse`  : skip both. The existing offscreen is blitted unchanged.
    ///   The orchestrator must NOT have called
    ///   `begin_layer(Layer::Content)` this frame.
    ///
    /// `overlay`:
    /// - `Redraw` : upload the freshly pushed overlay geometry and re-issue
    ///   the draw call.
    /// - `Reuse`  : reuse the previously uploaded GPU buffer (idle frames).
    ///
    /// The blit pass always runs because the swapchain texture is a fresh,
    /// undefined-content surface each frame.
    pub fn flush_dirty_rect(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        swapchain_view: &wgpu::TextureView,
        cells:   ContentRepaint,
        overlay: OverlayRepaint,
    ) {
        // -- 0. Uniform + atlas + conditional batch uploads. ------------------
        let vp: [f32; 4] = [self.width, self.height, 0.0, 0.0];
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(&vp));
        self.atlas.flush_uploads(queue);
        // Only uploads if the matching `begin_rebuild` was called this frame.
        self.cells  .upload_if_rebuilding(device, queue, "cells");
        if matches!(overlay, OverlayRepaint::Redraw) {
            self.overlay.upload_if_rebuilding(device, queue, "overlay");
        }

        // -- 1. Cells layer: shift + render to the destination offscreen. -----
        let cells_dirty = !matches!(cells, ContentRepaint::Reuse);
        let dst_idx = if cells_dirty {
            1 - self.current_offscreen_idx
        } else {
            self.current_offscreen_idx
        };

        if let ContentRepaint::Scroll { dx_px, dy_px } = cells {
            let src = &self.offscreens[self.current_offscreen_idx].texture;
            let dst = &self.offscreens[dst_idx].texture;
            copy_offscreen_with_offset(
                encoder, src, dst,
                self.width as u32, self.height as u32,
                dx_px, dy_px,
            );
        }

        if cells_dirty {
            let load_op = match cells {
                ContentRepaint::Full          => wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                ContentRepaint::Scroll { .. } => wgpu::LoadOp::Load,
                ContentRepaint::Reuse         => wgpu::LoadOp::Load,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cells_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.offscreens[dst_idx].view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: load_op, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            let q_count = self.cells.drawn_quad_indices;
            let g_count = self.cells.drawn_glyph_indices;
            if q_count > 0 || g_count > 0 {
                pass.set_bind_group(0, &self.uniform_bg, &[]);
                if q_count > 0 {
                    pass.set_pipeline(&self.quad_pipeline);
                    pass.set_vertex_buffer(0, self.cells.quad_vb.slice(..));
                    pass.set_index_buffer(self.cells.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..q_count, 0, 0..1);
                }
                if g_count > 0 {
                    pass.set_pipeline(&self.glyph_pipeline);
                    pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.cells.glyph_vb.slice(..));
                    pass.set_index_buffer(self.cells.glyph_ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..g_count, 0, 0..1);
                }
            }
            self.current_offscreen_idx = dst_idx;
        }

        // -- 2. Blit the persistent cells offscreen to the swapchain. ---------
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: swapchain_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &self.blit_bind_groups[self.current_offscreen_idx], &[]);
            pass.draw(0..3, 0..1);
        }

        // -- 3. Overlay layer: render on top of the swapchain. ----------------
        let overlay_q = self.overlay.drawn_quad_indices;
        let overlay_g = self.overlay.drawn_glyph_indices;
        if overlay_q > 0 || overlay_g > 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: swapchain_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.uniform_bg, &[]);
            if overlay_q > 0 {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_vertex_buffer(0, self.overlay.quad_vb.slice(..));
                pass.set_index_buffer(self.overlay.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..overlay_q, 0, 0..1);
            }
            if overlay_g > 0 {
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                pass.set_vertex_buffer(0, self.overlay.glyph_vb.slice(..));
                pass.set_index_buffer(self.overlay.glyph_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..overlay_g, 0, 0..1);
            }
        }
    }
}

// ============================================================================
// WgpuSurface
// ============================================================================

/// Frame-scoped `DrawSurface` backed by `QuadRenderer`. Push routing follows
/// `renderer.active_layer`, set by the caller before each phase.
pub struct WgpuSurface<'r> {
    renderer:   &'r mut QuadRenderer,
    clip_stack: Vec<Rect>,
}

impl<'r> WgpuSurface<'r> {
    pub fn new(renderer: &'r mut QuadRenderer) -> Self {
        Self {
            renderer,
            clip_stack: Vec::new(),
        }
    }

    /// Switch the surface (and underlying renderer) to a different layer,
    /// arming it for rebuild this frame. Clears the clip stack since clips
    /// don't cross layer boundaries.
    pub fn begin_layer(&mut self, layer: Layer) {
        self.renderer.begin_layer(layer);
        self.clip_stack.clear();
    }

    fn active_clip(&self) -> Option<Rect> {
        self.clip_stack.last().copied()
    }

    fn clip_rect(&self, rect: Rect) -> Rect {
        match self.active_clip() {
            Some(c) => c.intersect(rect),
            None => rect,
        }
    }
}

impl DrawSurface for WgpuSurface<'_> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let r = self.clip_rect(rect);
        self.renderer.push_quad(r, color);
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64) {
        let hw = width * 0.5;
        // 4 edge quads with mitered corners.
        self.fill_rect(
            Rect::new(rect.x0 - hw, rect.y0 - hw, rect.x1 + hw, rect.y0 + hw),
            color,
        );
        self.fill_rect(
            Rect::new(rect.x0 - hw, rect.y1 - hw, rect.x1 + hw, rect.y1 + hw),
            color,
        );
        self.fill_rect(
            Rect::new(rect.x0 - hw, rect.y0 + hw, rect.x0 + hw, rect.y1 - hw),
            color,
        );
        self.fill_rect(
            Rect::new(rect.x1 - hw, rect.y0 + hw, rect.x1 + hw, rect.y1 - hw),
            color,
        );
    }

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        // Conservative clip: reject lines fully outside the clip rect.
        if let Some(clip) = self.active_clip() {
            let min_x = x0.min(x1);
            let max_x = x0.max(x1);
            let min_y = y0.min(y1);
            let max_y = y0.max(y1);
            if max_x < clip.x0 || min_x > clip.x1 || max_y < clip.y0 || min_y > clip.y1 {
                return;
            }
        }
        self.renderer.push_line(
            x0 as f32,
            y0 as f32,
            x1 as f32,
            y1 as f32,
            color,
            width as f32,
        );
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return, // gradients: Phase 3
        };

        let parley_run = run.run();
        let font_data = parley_run.font();
        let font_size = parley_run.font_size();
        let coords = parley_run.normalized_coords(); // &[NormalizedCoord] -- no alloc
        let clip = self.active_clip();

        let mut pen_x = 0.0f32;
        for glyph in run.glyphs() {
            let gx = pen_x + glyph.x;
            let gy = glyph.y;
            pen_x += glyph.advance;

            let slot = match self.renderer.atlas.get_or_rasterize(
                font_data,
                font_size,
                coords,
                glyph.id as u16,
            ) {
                Some(s) => s,
                None => continue,
            };

            if slot.pixel_width == 0 || slot.pixel_height == 0 {
                continue; // whitespace / invisible glyph
            }

            // Transform the glyph origin from run-local to screen space.
            let origin = transform * Point::new(gx as f64, gy as f64);

            // Bitmap top-left in screen space.
            // bearing_top is positive upward (font convention), so subtract.
            let sx = origin.x as f32 + slot.bearing_left as f32;
            let sy = origin.y as f32 - slot.bearing_top as f32;
            let sw = slot.pixel_width as f32;
            let sh = slot.pixel_height as f32;

            // Reject if fully outside clip rect.
            if let Some(clip) = clip
                && (sx + sw < clip.x0 as f32
                    || sx > clip.x1 as f32
                    || sy + sh < clip.y0 as f32
                    || sy > clip.y1 as f32)
            {
                continue;
            }

            self.renderer
                .push_glyph(sx, sy, sw, sh, slot.uv_min, slot.uv_max, color);
        }
    }

    fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.active_clip() {
            Some(p) => p.intersect(rect),
            None => rect,
        };
        self.clip_stack.push(clipped);
    }

    fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn color_f32(c: Color) -> [f32; 4] {
    // peniko::AlphaColor<Srgb> stores [r, g, b, a] as f32 0..=1.
    c.components
}

/// Recreate `buf` if `needed_bytes` exceeds its current size.
/// Allocates at 2x the needed size to amortize future resizes.
fn grow_buffer_if_needed(
    device: &wgpu::Device,
    buf: &mut wgpu::Buffer,
    needed_bytes: usize,
    usage: wgpu::BufferUsages,
    name: &'static str,
    label_prefix: &str,
) {
    if needed_bytes as u64 > buf.size() {
        let new_size = (needed_bytes * 2).next_power_of_two() as u64;
        *buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_{name}")),
            size: new_size,
            usage,
            mapped_at_creation: false,
        });
    }
}

fn make_blit_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    label: &str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Encode a `copy_texture_to_texture` that shifts `src` by `(dx_px, dy_px)`
/// pixels into `dst`. Negative deltas shift content left/up; positive shift
/// right/down. The strip exposed by the shift is *not* cleared — the caller
/// must render fresh geometry over it in the subsequent cells pass.
///
/// WebGL2 forbids same-texture src/dst, hence the ping-pong pair: `src` and
/// `dst` must be distinct GPU textures.
fn copy_offscreen_with_offset(
    encoder: &mut wgpu::CommandEncoder,
    src: &wgpu::Texture,
    dst: &wgpu::Texture,
    width:  u32,
    height: u32,
    dx_px:  i32,
    dy_px:  i32,
) {
    // Compute the overlapping region: source rect that has a valid mapping
    // into the destination. Out-of-range deltas leave nothing to copy.
    let dx_abs = dx_px.unsigned_abs();
    let dy_abs = dy_px.unsigned_abs();
    if dx_abs >= width || dy_abs >= height {
        return;
    }
    let copy_w = width  - dx_abs;
    let copy_h = height - dy_abs;

    let (src_x, dst_x) = if dx_px >= 0 {
        // Content shifts right: take src starting at x=0, place at dst x=dx.
        (0u32, dx_abs)
    } else {
        // Content shifts left: take src starting at x=dx_abs, place at dst x=0.
        (dx_abs, 0u32)
    };
    let (src_y, dst_y) = if dy_px >= 0 {
        (0u32, dy_abs)
    } else {
        (dy_abs, 0u32)
    };

    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: src,
            mip_level: 0,
            origin: wgpu::Origin3d { x: src_x, y: src_y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d { x: dst_x, y: dst_y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width:                 copy_w,
            height:                copy_h,
            depth_or_array_layers: 1,
        },
    );
}
