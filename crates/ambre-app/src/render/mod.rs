// ambre-app/src/render/mod.rs
// ============================================================================
// CPU-batched renderer backed by wgpu vertex/fragment shaders.
//
// No compute shaders -- every draw call maps to plain triangles + textured quads.
//
// Two pipelines, flushed in order each frame:
//   1. Quad pipeline  -- solid-color filled/stroked rectangles and lines.
//   2. Glyph pipeline -- alpha-mask glyphs from the GlyphAtlas (R8Unorm).
//
// Public API:
//   QuadRenderer  -- owns both pipelines, GlyphAtlas, and CPU-side batches.
//   WgpuSurface   -- frame-scoped DrawSurface. Holds the clip stack.
//                    Clipping is performed CPU-side before pushing vertices.
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
    pos:   [f32; 2],
    color: [f32; 4],
}

const QUAD_VERTEX_SIZE: usize = std::mem::size_of::<QuadVertex>();
const INITIAL_QUADS:    usize = 8192;

// ============================================================================
// Glyph pipeline
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlyphVertex {
    pos:   [f32; 2],
    uv:    [f32; 2],
    color: [f32; 4],
}

const GLYPH_VERTEX_SIZE: usize = std::mem::size_of::<GlyphVertex>();
const INITIAL_GLYPHS:    usize = 4096;

// ============================================================================
// GlyphAtlas
// ============================================================================

const ATLAS_SIZE: u32 = 2048;
// 1-pixel padding between glyphs prevents bilinear bleed at atlas boundaries.
const ATLAS_PAD: u32 = 1;

#[derive(Clone, PartialEq, Eq, Hash)]
struct GlyphKey {
    font_id:   u64, // Blob::id() -- stable identifier per registered font
    glyph_id:  u16,
    size_x100: u32, // font_size * 100 to avoid floats in hash key
}

/// UV rect and pixel-space bearing for one atlas entry.
#[derive(Clone, Copy)]
pub struct AtlasSlot {
    pub uv_min:        [f32; 2],
    pub uv_max:        [f32; 2],
    pub pixel_width:   u32,
    pub pixel_height:  u32,
    /// Horizontal offset from the glyph origin to the bitmap left edge (px).
    pub bearing_left:  i32,
    /// Vertical offset from the glyph origin (baseline) to the bitmap top (px).
    /// Positive = above baseline. Apply as: `screen_y = origin_y - bearing_top`.
    pub bearing_top:   i32,
}

struct ShelfPacker {
    cur_x:      u32,
    cur_y:      u32,  // top of the current shelf
    shelf_h:    u32,  // max glyph height on the current shelf
    atlas_size: u32,
}

impl ShelfPacker {
    fn new(atlas_size: u32) -> Self {
        Self { cur_x: 0, cur_y: 0, shelf_h: 0, atlas_size }
    }

    fn reset(&mut self) {
        self.cur_x   = 0;
        self.cur_y   = 0;
        self.shelf_h = 0;
    }

    /// Try to allocate a region of `(w, h)` pixels.
    /// Returns the top-left pixel coordinate on success, or `None` if the
    /// atlas is full (caller should reset the atlas and retry).
    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let w_padded = w + ATLAS_PAD;
        // Advance to next shelf if this glyph doesn't fit horizontally.
        if self.cur_x + w_padded > self.atlas_size {
            self.cur_y   += self.shelf_h + ATLAS_PAD;
            self.cur_x   = 0;
            self.shelf_h = 0;
        }
        // Check vertical overflow.
        if self.cur_y + h > self.atlas_size {
            return None;
        }
        let (x, y) = (self.cur_x, self.cur_y);
        self.cur_x   += w_padded;
        self.shelf_h = self.shelf_h.max(h);
        Some((x, y))
    }
}

/// CPU + GPU glyph atlas.
///
/// Glyphs are rasterized via Swash into a 2048x2048 R8Unorm GPU texture.
/// Pending CPU-to-GPU uploads are batched and flushed once per frame.
pub struct GlyphAtlas {
    pub texture:  wgpu::Texture,
    pub view:     wgpu::TextureView,
    cache:        FxHashMap<GlyphKey, AtlasSlot>,
    packer:       ShelfPacker,
    scale_cx:     swash::scale::ScaleContext,
    /// Queued texture uploads: (x, y, w, h, alpha_data).
    pending:      Vec<(u32, u32, u32, u32, Vec<u8>)>,
    needs_clear:  bool,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("glyph_atlas"),
            size:            wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::R8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        let view = texture.create_view(&Default::default());

        Self {
            texture,
            view,
            cache:       FxHashMap::default(),
            packer:      ShelfPacker::new(ATLAS_SIZE),
            scale_cx:    swash::scale::ScaleContext::new(),
            pending:     Vec::new(),
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
        font_data:         &parley::FontData,
        font_size:         f32,
        normalized_coords: &[swash::NormalizedCoord],
        glyph_id:          u16,
    ) -> Option<AtlasSlot> {
        let key = GlyphKey {
            font_id:   font_data.data.id(),
            glyph_id,
            size_x100: (font_size * 100.0) as u32,
        };

        if let Some(&slot) = self.cache.get(&key) {
            return Some(slot);
        }

        // Rasterize via Swash.
        let font_ref = swash::FontRef::from_index(
            font_data.data.as_ref(),
            font_data.index as usize,
        )?;

        let image = swash::scale::Render::new(&[
            swash::scale::Source::ColorOutline(0),
            swash::scale::Source::Outline,
        ])
        .format(swash::zeno::Format::Alpha)
        .render(
            &mut self.scale_cx
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
                uv_min:       [0.0; 2],
                uv_max:       [0.0; 2],
                pixel_width:  0,
                pixel_height: 0,
                bearing_left: image.placement.left,
                bearing_top:  image.placement.top,
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
            uv_min:       [ax as f32 * inv, ay as f32 * inv],
            uv_max:       [(ax + w) as f32 * inv, (ay + h) as f32 * inv],
            pixel_width:  w,
            pixel_height: h,
            bearing_left: image.placement.left,
            bearing_top:  image.placement.top,
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
                    texture:   &self.texture,
                    mip_level: 0,
                    origin:    wgpu::Origin3d { x, y, z: 0 },
                    aspect:    wgpu::TextureAspect::All,
                },
                &upload_data,
                wgpu::TexelCopyBufferLayout {
                    offset:         0,
                    bytes_per_row:  Some(aligned_bpr),
                    rows_per_image: None,
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
    }
}

// ============================================================================
// QuadRenderer
// ============================================================================

/// Owns both wgpu pipelines, the GlyphAtlas, and the per-frame CPU batches.
///
/// Frame protocol:
///   1. `begin_frame(w, h)` -- clear batches, update viewport.
///   2. Push geometry via `WgpuSurface` (calls `push_quad`, `push_line`,
///      `push_glyph` through `atlas_get_or_rasterize`).
///   3. `flush(device, queue, encoder, target_view)` -- upload + two draw calls.
pub struct QuadRenderer {
    // -- Quad pipeline --
    quad_pipeline: wgpu::RenderPipeline,
    quad_vb:       wgpu::Buffer,
    quad_ib:       wgpu::Buffer,
    quad_vertices: Vec<QuadVertex>,
    quad_indices:  Vec<u32>,

    // -- Glyph pipeline --
    glyph_pipeline:  wgpu::RenderPipeline,
    glyph_vb:        wgpu::Buffer,
    glyph_ib:        wgpu::Buffer,
    glyph_vertices:  Vec<GlyphVertex>,
    glyph_indices:   Vec<u32>,
    /// Bind group holding the atlas texture + sampler (group 1).
    atlas_bind_group: wgpu::BindGroup,

    // -- Shared state --
    pub atlas:   GlyphAtlas,
    uniform_buf: wgpu::Buffer,
    /// Bind group for the viewport uniform (group 0), shared by both pipelines.
    uniform_bg:  wgpu::BindGroup,
    width:       f32,
    height:      f32,
}

impl QuadRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        // -- Shared viewport uniform --
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("quad_uniforms"),
            size:               16, // vec2<f32> + 8-byte pad
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("uniform_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty:         wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }],
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("uniform_bg"),
            layout:  &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        // -- Quad pipeline --
        let quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("solid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("solid.wgsl").into()),
        });
        let quad_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("quad_pl"),
            bind_group_layouts: &[&uniform_bgl],
            immediate_size:     0,
        });
        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("quad"),
            layout: Some(&quad_pl_layout),
            vertex: wgpu::VertexState {
                module:              &quad_shader,
                entry_point:         Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: QUAD_VERTEX_SIZE as u64,
                    step_mode:    wgpu::VertexStepMode::Vertex,
                    attributes:   &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module:              &quad_shader,
                entry_point:         Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive:      wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil:  None,
            multisample:    wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache:          None,
        });

        // -- Glyph pipeline --
        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("glyph"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glyph.wgsl").into()),
        });
        let atlas_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("atlas_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count:      None,
                },
            ],
        });
        let glyph_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("glyph_pl"),
            bind_group_layouts: &[&uniform_bgl, &atlas_bgl],
            immediate_size:     0,
        });
        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("glyph"),
            layout: Some(&glyph_pl_layout),
            vertex: wgpu::VertexState {
                module:              &glyph_shader,
                entry_point:         Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: GLYPH_VERTEX_SIZE as u64,
                    step_mode:    wgpu::VertexStepMode::Vertex,
                    attributes:   &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module:              &glyph_shader,
                entry_point:         Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive:      wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil:  None,
            multisample:    wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache:          None,
        });

        // -- GlyphAtlas --
        let atlas = GlyphAtlas::new(device);

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:             Some("atlas_sampler"),
            address_mode_u:    wgpu::AddressMode::ClampToEdge,
            address_mode_v:    wgpu::AddressMode::ClampToEdge,
            mag_filter:        wgpu::FilterMode::Linear,
            min_filter:        wgpu::FilterMode::Linear,
            mipmap_filter:     wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("atlas_bg"),
            layout:  &atlas_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&atlas_sampler) },
            ],
        });

        // -- Pre-allocate vertex/index buffers --
        let quad_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("quad_vb"),
            size:               (INITIAL_QUADS  * 4 * QUAD_VERTEX_SIZE)  as u64,
            usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("quad_ib"),
            size:               (INITIAL_QUADS  * 6 * 4) as u64,
            usage:              wgpu::BufferUsages::INDEX  | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("glyph_vb"),
            size:               (INITIAL_GLYPHS * 4 * GLYPH_VERTEX_SIZE) as u64,
            usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("glyph_ib"),
            size:               (INITIAL_GLYPHS * 6 * 4) as u64,
            usage:              wgpu::BufferUsages::INDEX  | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            quad_pipeline,
            quad_vb,
            quad_ib,
            quad_vertices: Vec::with_capacity(INITIAL_QUADS  * 4),
            quad_indices:  Vec::with_capacity(INITIAL_QUADS  * 6),
            glyph_pipeline,
            glyph_vb,
            glyph_ib,
            glyph_vertices: Vec::with_capacity(INITIAL_GLYPHS * 4),
            glyph_indices:  Vec::with_capacity(INITIAL_GLYPHS * 6),
            atlas_bind_group,
            atlas,
            uniform_buf,
            uniform_bg,
            width:  1.0,
            height: 1.0,
        }
    }

    pub fn begin_frame(&mut self, width: f32, height: f32) {
        self.quad_vertices.clear();
        self.quad_indices.clear();
        self.glyph_vertices.clear();
        self.glyph_indices.clear();
        self.atlas.begin_frame();
        self.width  = width;
        self.height = height;
    }

    // -- Quad geometry --------------------------------------------------------

    pub fn push_quad(&mut self, rect: Rect, color: Color) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 { return; }
        let c  = color_f32(color);
        let x0 = rect.x0 as f32;
        let y0 = rect.y0 as f32;
        let x1 = rect.x1 as f32;
        let y1 = rect.y1 as f32;
        let base = self.quad_vertices.len() as u32;
        self.quad_vertices.extend_from_slice(&[
            QuadVertex { pos: [x0, y0], color: c },
            QuadVertex { pos: [x1, y0], color: c },
            QuadVertex { pos: [x1, y1], color: c },
            QuadVertex { pos: [x0, y1], color: c },
        ]);
        self.quad_indices.extend_from_slice(&[base, base+1, base+2, base, base+2, base+3]);
    }

    pub fn push_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color, width: f32) {
        let dx  = x1 - x0;
        let dy  = y1 - y0;
        let len = dx.hypot(dy);
        if len < 0.001 { return; }
        let hw  = width * 0.5;
        let nx  = -dy / len * hw;
        let ny  =  dx / len * hw;
        let c   = color_f32(color);
        let base = self.quad_vertices.len() as u32;
        self.quad_vertices.extend_from_slice(&[
            QuadVertex { pos: [x0 + nx, y0 + ny], color: c },
            QuadVertex { pos: [x0 - nx, y0 - ny], color: c },
            QuadVertex { pos: [x1 - nx, y1 - ny], color: c },
            QuadVertex { pos: [x1 + nx, y1 + ny], color: c },
        ]);
        self.quad_indices.extend_from_slice(&[base, base+1, base+2, base, base+2, base+3]);
    }

    // -- Glyph geometry -------------------------------------------------------

    pub fn push_glyph(
        &mut self,
        x: f32, y: f32, w: f32, h: f32,
        uv_min: [f32; 2], uv_max: [f32; 2],
        color: Color,
    ) {
        if w <= 0.0 || h <= 0.0 { return; }
        let c    = color_f32(color);
        let base = self.glyph_vertices.len() as u32;
        self.glyph_vertices.extend_from_slice(&[
            GlyphVertex { pos: [x,     y    ], uv: [uv_min[0], uv_min[1]], color: c },
            GlyphVertex { pos: [x + w, y    ], uv: [uv_max[0], uv_min[1]], color: c },
            GlyphVertex { pos: [x + w, y + h], uv: [uv_max[0], uv_max[1]], color: c },
            GlyphVertex { pos: [x,     y + h], uv: [uv_min[0], uv_max[1]], color: c },
        ]);
        self.glyph_indices.extend_from_slice(&[base, base+1, base+2, base, base+2, base+3]);
    }

    // -- Frame flush ----------------------------------------------------------

    /// Encode a render pass for this frame.
    ///
    /// `upload` controls whether CPU→GPU buffer transfers are performed:
    /// - `true`  (dirty frame): upload uniform + atlas + vertex/index buffers.
    /// - `false` (idle frame):  skip all uploads; reuse the GPU buffers from the
    ///   last dirty frame. The CPU-side Vecs still hold the previous frame's data
    ///   so `has_quads` / `has_glyphs` remain accurate.
    ///
    /// Both paths encode the same draw commands, so the displayed content is
    /// identical on idle frames with zero `write_buffer` overhead.
    pub fn flush(
        &mut self,
        device:      &wgpu::Device,
        queue:       &wgpu::Queue,
        encoder:     &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        upload:      bool,
    ) {
        let has_quads  = !self.quad_indices.is_empty();
        let has_glyphs = !self.glyph_indices.is_empty();

        if upload {
            // Upload viewport uniform.
            let vp: [f32; 4] = [self.width, self.height, 0.0, 0.0];
            queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(&vp));

            // Upload pending atlas glyph bitmaps.
            self.atlas.flush_uploads(queue);

            if has_quads {
                grow_buffer_if_needed(device, &mut self.quad_vb, self.quad_vertices.len() * QUAD_VERTEX_SIZE, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, "quad_vb");
                grow_buffer_if_needed(device, &mut self.quad_ib, self.quad_indices.len()  * 4,                wgpu::BufferUsages::INDEX  | wgpu::BufferUsages::COPY_DST, "quad_ib");
                queue.write_buffer(&self.quad_vb, 0, bytemuck::cast_slice(&self.quad_vertices));
                queue.write_buffer(&self.quad_ib, 0, bytemuck::cast_slice(&self.quad_indices));
            }
            if has_glyphs {
                grow_buffer_if_needed(device, &mut self.glyph_vb, self.glyph_vertices.len() * GLYPH_VERTEX_SIZE, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, "glyph_vb");
                grow_buffer_if_needed(device, &mut self.glyph_ib, self.glyph_indices.len()  * 4,                  wgpu::BufferUsages::INDEX  | wgpu::BufferUsages::COPY_DST, "glyph_ib");
                queue.write_buffer(&self.glyph_vb, 0, bytemuck::cast_slice(&self.glyph_vertices));
                queue.write_buffer(&self.glyph_ib, 0, bytemuck::cast_slice(&self.glyph_indices));
            }
        }

        // Always open a render pass -- even with zero draw calls -- to guarantee
        // the surface is cleared to a known colour on every frame. Without this,
        // the surface texture stays uninitialized on first use (shows as black or
        // garbage on some drivers/iGPUs).
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           target_view,
                resolve_target: None,
                depth_slice:    None,
                ops:            wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        if !has_quads && !has_glyphs {
            return;
        }

        pass.set_bind_group(0, &self.uniform_bg, &[]);

        if has_quads {
            pass.set_pipeline(&self.quad_pipeline);
            pass.set_vertex_buffer(0, self.quad_vb.slice(..));
            pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.quad_indices.len() as u32, 0, 0..1);
        }

        if has_glyphs {
            pass.set_pipeline(&self.glyph_pipeline);
            pass.set_bind_group(1, &self.atlas_bind_group, &[]);
            pass.set_vertex_buffer(0, self.glyph_vb.slice(..));
            pass.set_index_buffer(self.glyph_ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.glyph_indices.len() as u32, 0, 0..1);
        }
    }
}

// ============================================================================
// WgpuSurface
// ============================================================================

/// Frame-scoped `DrawSurface` backed by `QuadRenderer`.
pub struct WgpuSurface<'r> {
    renderer:   &'r mut QuadRenderer,
    clip_stack: Vec<Rect>,
}

impl<'r> WgpuSurface<'r> {
    pub fn new(renderer: &'r mut QuadRenderer) -> Self {
        Self { renderer, clip_stack: Vec::new() }
    }

    fn active_clip(&self) -> Option<Rect> {
        self.clip_stack.last().copied()
    }

    fn clip_rect(&self, rect: Rect) -> Rect {
        match self.active_clip() {
            Some(c) => c.intersect(rect),
            None    => rect,
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
        self.fill_rect(Rect::new(rect.x0 - hw, rect.y0 - hw, rect.x1 + hw, rect.y0 + hw), color);
        self.fill_rect(Rect::new(rect.x0 - hw, rect.y1 - hw, rect.x1 + hw, rect.y1 + hw), color);
        self.fill_rect(Rect::new(rect.x0 - hw, rect.y0 + hw, rect.x0 + hw, rect.y1 - hw), color);
        self.fill_rect(Rect::new(rect.x1 - hw, rect.y0 + hw, rect.x1 + hw, rect.y1 - hw), color);
    }

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        // Conservative clip: reject lines fully outside the clip rect.
        if let Some(clip) = self.active_clip() {
            let min_x = x0.min(x1); let max_x = x0.max(x1);
            let min_y = y0.min(y1); let max_y = y0.max(y1);
            if max_x < clip.x0 || min_x > clip.x1 || max_y < clip.y0 || min_y > clip.y1 {
                return;
            }
        }
        self.renderer.push_line(x0 as f32, y0 as f32, x1 as f32, y1 as f32, color, width as f32);
    }

    fn draw_glyphs(
        &mut self,
        run:       &GlyphRun<'_, ParleyBrush>,
        brush:     &Brush,
        transform: Affine,
    ) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _               => return, // gradients: Phase 3
        };

        let parley_run = run.run();
        let font_data  = parley_run.font();
        let font_size  = parley_run.font_size();
        let coords     = parley_run.normalized_coords(); // &[NormalizedCoord] -- no alloc
        let clip             = self.active_clip();

        let mut pen_x = 0.0f32;
        for glyph in run.glyphs() {
            let gx = pen_x + glyph.x;
            let gy = glyph.y;
            pen_x += glyph.advance;

            let slot = match self.renderer.atlas.get_or_rasterize(
                font_data, font_size, &coords, glyph.id as u16,
            ) {
                Some(s) => s,
                None    => continue,
            };

            if slot.pixel_width == 0 || slot.pixel_height == 0 {
                continue; // whitespace / invisible glyph
            }

            // Transform the glyph origin from run-local to screen space.
            let origin = transform * Point::new(gx as f64, gy as f64);

            // Bitmap top-left in screen space.
            // bearing_top is positive upward (font convention), so subtract.
            let sx = origin.x as f32 + slot.bearing_left as f32;
            let sy = origin.y as f32 - slot.bearing_top  as f32;
            let sw = slot.pixel_width  as f32;
            let sh = slot.pixel_height as f32;

            // Reject if fully outside clip rect.
            if let Some(clip) = clip {
                if sx + sw < clip.x0 as f32 || sx > clip.x1 as f32
                || sy + sh < clip.y0 as f32 || sy > clip.y1 as f32 {
                    continue;
                }
            }

            self.renderer.push_glyph(sx, sy, sw, sh, slot.uv_min, slot.uv_max, color);
        }
    }

    fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.active_clip() {
            Some(p) => p.intersect(rect),
            None    => rect,
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
    device:        &wgpu::Device,
    buf:           &mut wgpu::Buffer,
    needed_bytes:  usize,
    usage:         wgpu::BufferUsages,
    label:         &'static str,
) {
    if needed_bytes as u64 > buf.size() {
        let new_size = (needed_bytes * 2).next_power_of_two() as u64;
        *buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some(label),
            size:               new_size,
            usage,
            mapped_at_creation: false,
        });
    }
}
