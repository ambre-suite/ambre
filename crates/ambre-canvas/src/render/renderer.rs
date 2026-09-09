// ambre-canvas/src/render/renderer.rs
// ============================================================================
// QuadRenderer: owns the four wgpu pipelines (quad, glyph, color glyph, blit),
// the GlyphAtlas, the ping-pong offscreen pair for the persistent cells layer, and
// the per-frame dirty-rect flush. Also the offscreen target and the repaint
// plan enums. The `include_str!("*.wgsl")` calls resolve against this dir.
// ============================================================================

use super::*;
use ambre_draw::{Color, ImageSource, Rect};
use std::collections::HashMap;

// ============================================================================
// Offscreen target
// ============================================================================

struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
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
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
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
    quad_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    /// Full-color glyph pipeline (color emoji): samples the RGBA color atlas
    /// and outputs the texel directly, ignoring the vertex tint.
    color_glyph_pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,

    // -- Shared uniform (viewport size) --
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,

    // -- Glyph atlas + its bind groups (mask + color) --
    atlas_bind_group: wgpu::BindGroup,
    color_atlas_bind_group: wgpu::BindGroup,
    /// Kept so the color bind group can be rebuilt when the lazily-allocated
    /// color atlas texture grows from its placeholder to full size.
    atlas_bgl: wgpu::BindGroupLayout,
    atlas_sampler: wgpu::Sampler,
    pub atlas: GlyphAtlas,

    // -- Uploaded image textures, keyed by the caller's `ImageKey` --
    image_sampler: wgpu::Sampler,
    images: ImageTextures,

    // -- Per-layer batches --
    cells: LayerBatches,
    overlay: LayerBatches,
    active_layer: Layer,

    // -- Ping-pong offscreen pair for the cells layer --
    offscreen_format: wgpu::TextureFormat,
    offscreens: [OffscreenTarget; 2],
    blit_bind_groups: [wgpu::BindGroup; 2],
    blit_bgl: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    /// Index of the offscreen that holds the latest valid CELLS pixels.
    /// After a Full/Scroll repaint, we render into the OTHER index and flip.
    current_offscreen_idx: usize,

    // -- Viewport state --
    width: f32,
    height: f32,
}

/// How much VRAM the uploaded pictures may hold together, mip chains included.
///
/// The wgpu row of `ambre-spec-budgets.md section 6` caps textures and buffers
/// at 20 MB, of which the two glyph atlases already claim up to 20; pictures are
/// charged their own ceiling rather than sharing an unstated remainder. At four
/// bytes a texel a full budget is roughly six 1024x1024 photographs, and the
/// chain adds a third to each.
const IMAGE_VRAM_BUDGET: usize = 24 * 1024 * 1024;

/// One cached picture: what a draw binds, and what holding it costs.
struct CacheEntry<T> {
    payload: T,
    bytes: usize,
    /// The size it was uploaded at. A key reused at a different size is content
    /// that changed behind its identity, so the entry is replaced rather than
    /// stretched (`ImageSource::key`).
    width: u32,
    height: u32,
    /// Frame ordinal of the last draw that bound it, for eviction.
    last_used: u64,
}

/// The uploaded pictures, bounded by `IMAGE_VRAM_BUDGET` and evicted
/// least-recently-drawn first.
///
/// This is a *texture* cache, not the decoded-pixel cache `CANVAS-007` calls
/// for: that one is CPU-side, shared across applications, and still unbuilt.
/// The two are not interchangeable -- pixels feed an upload, and what is bound
/// per draw is what lives here.
///
/// It is generic over what it holds so its policy -- when an entry is a hit,
/// what is dropped when the budget binds -- can be exercised on the host tier,
/// where no GPU adapter exists to mint a bind group
/// (`ambre-spec-testing.md section 2`).
struct ImageCache<T> {
    map: HashMap<ImageKey, CacheEntry<T>>,
    bytes: usize,
    budget: usize,
    /// Monotonic per-frame counter, stamped onto an entry when it is drawn.
    clock: u64,
}

impl<T> ImageCache<T> {
    fn new(budget: usize) -> Self {
        Self {
            map: HashMap::new(),
            bytes: 0,
            budget,
            clock: 0,
        }
    }

    /// The payload for `key`, if these exact pixels are already resident. Stamps
    /// the entry so eviction sees it as recently used.
    fn get(&mut self, key: ImageKey, width: u32, height: u32) -> Option<&T> {
        let clock = self.clock;
        let entry = self.map.get_mut(&key)?;
        if entry.width != width || entry.height != height {
            return None;
        }
        entry.last_used = clock;
        Some(&entry.payload)
    }

    /// The payload for a run being encoded. Unlike `get` this asks no question
    /// about size: the run was recorded against the entry as uploaded, and an
    /// entry evicted in between simply is not there.
    fn bound(&self, key: ImageKey) -> Option<&T> {
        self.map.get(&key).map(|e| &e.payload)
    }

    /// Insert an upload, evicting the least recently drawn entries until the
    /// budget holds again. An image larger than the whole budget is stored
    /// anyway -- refusing it would paint nothing at all, and the next insert
    /// evicts it.
    fn insert(&mut self, key: ImageKey, entry: CacheEntry<T>) {
        if let Some(old) = self.map.remove(&key) {
            self.bytes -= old.bytes;
        }
        self.bytes += entry.bytes;
        self.map.insert(key, entry);
        while self.bytes > self.budget && self.map.len() > 1 {
            let victim = self
                .map
                .iter()
                .min_by_key(|(_, v)| v.last_used)
                .map(|(k, _)| *k);
            match victim {
                Some(k) => {
                    if let Some(v) = self.map.remove(&k) {
                        self.bytes -= v.bytes;
                    }
                }
                None => break,
            }
        }
    }
}

type ImageTextures = ImageCache<wgpu::BindGroup>;

/// Record one quad's index range against the texture it needs bound.
///
/// The run in flight is extended when it already binds this texture and the new
/// range continues it; anything else opens a run. That is what makes ten copies
/// of one picture a single draw while keeping two different pictures in the
/// order the walk drew them -- a merge that ignored order would let the later
/// picture land under the earlier one.
fn extend_runs(runs: &mut Vec<ImageRun>, key: ImageKey, start: u32, end: u32) {
    match runs.last_mut() {
        Some(run) if run.key == key && run.end == start => run.end = end,
        _ => runs.push(ImageRun { key, start, end }),
    }
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

        // -- Color glyph pipeline (color emoji) --
        // Same pipeline layout (uniform + atlas bind groups) and vertex layout
        // as the mask glyph pipeline; only the shader differs (outputs the
        // sampled RGBA texel directly instead of tinting a coverage mask).
        let color_glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("color_glyph"),
            source: wgpu::ShaderSource::Wgsl(include_str!("color_glyph.wgsl").into()),
        });
        let color_glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("color_glyph"),
            layout: Some(&glyph_pl_layout),
            vertex: wgpu::VertexState {
                module: &color_glyph_shader,
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
                module: &color_glyph_shader,
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
        // Second bind group over the RGBA color atlas, same layout + sampler.
        let color_atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("color_atlas_bg"),
            layout: &atlas_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas.color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        // -- Image pipeline --
        // The same bind-group shape as the atlases (texture + sampler at group
        // 1), so one layout serves both; what differs is the sampler, which is
        // trilinear here because a picture is minified and a glyph never is.
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image"),
            source: wgpu::ShaderSource::Wgsl(include_str!("image.wgsl").into()),
        });
        let image_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("image_pl"),
            bind_group_layouts: &[&uniform_bgl, &atlas_bgl],
            immediate_size: 0,
        });
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("image"),
            layout: Some(&image_pl_layout),
            vertex: wgpu::VertexState {
                module: &image_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: IMAGE_VERTEX_SIZE as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
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

        // -- Image sampler --
        // Trilinear: linear within a mip level and linear between two, so a
        // picture shrinking under a zoom crosses levels without the visible step
        // a `Nearest` mipmap filter leaves. `ClampToEdge` keeps the border texel
        // from wrapping onto the opposite edge of the quad.
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
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
            make_blit_bg(
                device,
                &blit_bgl,
                &offscreens[0].view,
                &blit_sampler,
                "blit_bg_a",
            ),
            make_blit_bg(
                device,
                &blit_bgl,
                &offscreens[1].view,
                &blit_sampler,
                "blit_bg_b",
            ),
        ];

        let cells = LayerBatches::new(device, "cells");
        let overlay = LayerBatches::new(device, "overlay");

        Self {
            quad_pipeline,
            glyph_pipeline,
            color_glyph_pipeline,
            image_pipeline,
            blit_pipeline,
            uniform_buf,
            uniform_bg,
            atlas_bind_group,
            color_atlas_bind_group,
            atlas_bgl,
            atlas_sampler,
            image_sampler,
            images: ImageTextures::new(IMAGE_VRAM_BUDGET),
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
            width: 1.0,
            height: 1.0,
        }
    }

    /// Resize the ping-pong offscreens. Must be called whenever the canvas
    /// size changes. Both offscreens are reallocated and their content lost;
    /// the next frame must use `ContentRepaint::Full`.
    pub fn resize_offscreens(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.offscreens = [
            OffscreenTarget::new(
                device,
                self.offscreen_format,
                width,
                height,
                "cells_offscreen_a",
            ),
            OffscreenTarget::new(
                device,
                self.offscreen_format,
                width,
                height,
                "cells_offscreen_b",
            ),
        ];
        self.blit_bind_groups = [
            make_blit_bg(
                device,
                &self.blit_bgl,
                &self.offscreens[0].view,
                &self.blit_sampler,
                "blit_bg_a",
            ),
            make_blit_bg(
                device,
                &self.blit_bgl,
                &self.offscreens[1].view,
                &self.blit_sampler,
                "blit_bg_b",
            ),
        ];
        self.current_offscreen_idx = 0;
    }

    /// Per-frame init: ticks the atlas and updates the cached viewport size.
    /// Does NOT touch per-layer CPU batches — call `begin_layer` to clear and
    /// arm a layer for rebuild this frame; layers not begun keep their last
    /// uploaded GPU state and can be redrawn from cache (idle frames).
    pub fn begin_frame(&mut self, width: f32, height: f32) {
        self.atlas.begin_frame();
        // Advance the eviction clock so "least recently drawn" means a frame
        // count and not an arbitrary order: an image the walk stops drawing is
        // the first to go when the VRAM budget binds.
        self.images.clock = self.images.clock.wrapping_add(1);
        self.width = width;
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
            Layer::Content => &mut self.cells,
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
            QuadVertex {
                pos: [x0, y0],
                color: c,
            },
            QuadVertex {
                pos: [x1, y0],
                color: c,
            },
            QuadVertex {
                pos: [x1, y1],
                color: c,
            },
            QuadVertex {
                pos: [x0, y1],
                color: c,
            },
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
            QuadVertex {
                pos: [x0 + nx, y0 + ny],
                color: c,
            },
            QuadVertex {
                pos: [x0 - nx, y0 - ny],
                color: c,
            },
            QuadVertex {
                pos: [x1 - nx, y1 - ny],
                color: c,
            },
            QuadVertex {
                pos: [x1 + nx, y1 + ny],
                color: c,
            },
        ]);
        b.quad_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Append a pre-tessellated triangle mesh to the active layer's quad batch.
    ///
    /// `vertices` are screen-space positions and `indices` reference them
    /// locally (0-based); they are rebased onto the current batch. Used by
    /// `WgpuSurface::{fill_path, stroke_path}` after lyon tessellation -- path
    /// meshes share the solid-color quad pipeline (same `QuadVertex`, same
    /// `TriangleList` topology), so they need no pipeline of their own.
    pub fn push_mesh(&mut self, vertices: &[[f32; 2]], indices: &[u32], color: Color) {
        if vertices.is_empty() || indices.is_empty() {
            return;
        }
        let c = color_f32(color);
        let b = self.active_batches_mut();
        let base = b.quad_vertices.len() as u32;
        b.quad_vertices
            .extend(vertices.iter().map(|&pos| QuadVertex { pos, color: c }));
        b.quad_indices.extend(indices.iter().map(|&i| base + i));
    }

    // -- Images ---------------------------------------------------------------

    /// Append one textured quad for `image`, uploading its texture the first
    /// time this key is seen (`CANVAS-065`).
    ///
    /// `corners` are `[tl, tr, br, bl]` in screen space, already mapped through
    /// the active transform, so a rotated element rotates its picture with it
    /// rather than stretching an upright one into the box.
    ///
    /// A source whose buffer is shorter than its declared size draws nothing:
    /// the caller decoded it, so a short buffer is its bug, and uploading
    /// whatever follows in memory would put arbitrary bytes on screen.
    pub fn push_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &ImageSource<'_>,
        corners: [[f32; 2]; 4],
    ) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        let needed = (image.width as usize) * (image.height as usize) * 4;
        if image.pixels.len() < needed {
            return;
        }
        if self
            .images
            .get(image.key, image.width, image.height)
            .is_none()
        {
            let uploaded = self.upload_image(device, queue, image);
            self.images.insert(image.key, uploaded);
        }
        let key = image.key;
        let b = match self.active_layer {
            Layer::Content => &mut self.cells,
            Layer::Overlay => &mut self.overlay,
        };
        let base = b.image_vertices.len() as u32;
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for (i, c) in corners.iter().enumerate() {
            b.image_vertices.push(ImageVertex { pos: *c, uv: uv[i] });
        }
        let start = b.image_indices.len() as u32;
        b.image_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        let end = b.image_indices.len() as u32;
        extend_runs(&mut b.image_runs, key, start, end);
    }

    /// Upload one picture and its CPU-built mip chain, and mint the bind group
    /// the image pipeline binds per run.
    fn upload_image(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &ImageSource<'_>,
    ) -> CacheEntry<wgpu::BindGroup> {
        let chain = mipmap::build_chain(image.pixels, image.width, image.height);
        let levels = mipmap::level_count(image.width, image.height);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image_texture"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let write = |level: u32, pixels: &[u8], w: u32, h: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        };
        let level0 = (image.width as usize) * (image.height as usize) * 4;
        write(0, &image.pixels[..level0], image.width, image.height);
        let mut bytes = level0;
        for (i, level) in chain.iter().enumerate() {
            write(i as u32 + 1, &level.pixels, level.width, level.height);
            bytes += level.pixels.len();
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image_bg"),
            layout: &self.atlas_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.image_sampler),
                },
            ],
        });
        CacheEntry {
            payload: bind_group,
            bytes,
            width: image.width,
            height: image.height,
            last_used: self.images.clock,
        }
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
            GlyphVertex {
                pos: [x, y],
                uv: [uv_min[0], uv_min[1]],
                color: c,
            },
            GlyphVertex {
                pos: [x + w, y],
                uv: [uv_max[0], uv_min[1]],
                color: c,
            },
            GlyphVertex {
                pos: [x + w, y + h],
                uv: [uv_max[0], uv_max[1]],
                color: c,
            },
            GlyphVertex {
                pos: [x, y + h],
                uv: [uv_min[0], uv_max[1]],
                color: c,
            },
        ]);
        b.glyph_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Append a color-glyph quad (color emoji) to the active layer's color
    /// batch. Same geometry as `push_glyph`; the color pipeline samples the
    /// RGBA atlas and ignores the tint, so `color` is a placeholder here.
    #[allow(clippy::too_many_arguments)]
    pub fn push_color_glyph(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        uv_min: [f32; 2],
        uv_max: [f32; 2],
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let c = [0.0, 0.0, 0.0, 1.0];
        let b = self.active_batches_mut();
        let base = b.color_glyph_vertices.len() as u32;
        b.color_glyph_vertices.extend_from_slice(&[
            GlyphVertex {
                pos: [x, y],
                uv: [uv_min[0], uv_min[1]],
                color: c,
            },
            GlyphVertex {
                pos: [x + w, y],
                uv: [uv_max[0], uv_min[1]],
                color: c,
            },
            GlyphVertex {
                pos: [x + w, y + h],
                uv: [uv_max[0], uv_max[1]],
                color: c,
            },
            GlyphVertex {
                pos: [x, y + h],
                uv: [uv_min[0], uv_max[1]],
                color: c,
            },
        ]);
        b.color_glyph_indices.extend_from_slice(&[
            base,
            base + 1,
            base + 2,
            base,
            base + 2,
            base + 3,
        ]);
    }

    /// Append a coverage-mask glyph quad from four explicit corner positions
    /// (screen space, CCW from the top-left in glyph-local orientation), for a
    /// **rotated** glyph run: the caller has already mapped the quad corners
    /// through the run transform, so the glyph bitmap rotates with the text rather
    /// than staying upright (`toile-tools`). `corners` are
    /// `[tl, tr, br, bl]`, matching the `uv_min`/`uv_max` mapping of
    /// [`push_glyph`].
    pub fn push_glyph_quad(
        &mut self,
        corners: [[f32; 2]; 4],
        uv_min: [f32; 2],
        uv_max: [f32; 2],
        color: Color,
    ) {
        let c = color_f32(color);
        let uv = [
            [uv_min[0], uv_min[1]],
            [uv_max[0], uv_min[1]],
            [uv_max[0], uv_max[1]],
            [uv_min[0], uv_max[1]],
        ];
        let b = self.active_batches_mut();
        let base = b.glyph_vertices.len() as u32;
        for i in 0..4 {
            b.glyph_vertices.push(GlyphVertex {
                pos: corners[i],
                uv: uv[i],
                color: c,
            });
        }
        b.glyph_indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Rotated-quad counterpart of [`push_color_glyph`] (color emoji): same corner
    /// convention as [`push_glyph_quad`]; the color pipeline ignores the tint.
    pub fn push_color_glyph_quad(
        &mut self,
        corners: [[f32; 2]; 4],
        uv_min: [f32; 2],
        uv_max: [f32; 2],
    ) {
        let c = [0.0, 0.0, 0.0, 1.0];
        let uv = [
            [uv_min[0], uv_min[1]],
            [uv_max[0], uv_min[1]],
            [uv_max[0], uv_max[1]],
            [uv_min[0], uv_max[1]],
        ];
        let b = self.active_batches_mut();
        let base = b.color_glyph_vertices.len() as u32;
        for i in 0..4 {
            b.color_glyph_vertices.push(GlyphVertex {
                pos: corners[i],
                uv: uv[i],
                color: c,
            });
        }
        b.color_glyph_indices.extend_from_slice(&[
            base,
            base + 1,
            base + 2,
            base,
            base + 2,
            base + 3,
        ]);
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
        cells: ContentRepaint,
        overlay: OverlayRepaint,
    ) {
        // -- 0. Uniform + atlas + conditional batch uploads. ------------------
        let vp: [f32; 4] = [self.width, self.height, 0.0, 0.0];
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(&vp));
        // Grow the lazily-allocated color atlas on the first color glyph and
        // rebuild its bind group over the new texture view.
        if self.atlas.ensure_color_atlas(device) {
            self.color_atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("color_atlas_bg"),
                layout: &self.atlas_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.atlas.color_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                    },
                ],
            });
        }
        self.atlas.flush_uploads(queue);
        // Only uploads if the matching `begin_rebuild` was called this frame.
        self.cells.upload_if_rebuilding(device, queue, "cells");
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
                encoder,
                src,
                dst,
                self.width as u32,
                self.height as u32,
                dx_px,
                dy_px,
            );
        }

        if cells_dirty {
            let load_op = match cells {
                ContentRepaint::Full => wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                ContentRepaint::Scroll { .. } => wgpu::LoadOp::Load,
                ContentRepaint::Reuse => wgpu::LoadOp::Load,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cells_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.offscreens[dst_idx].view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: load_op,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let q_count = self.cells.drawn_quad_indices;
            let g_count = self.cells.drawn_glyph_indices;
            let cg_count = self.cells.drawn_color_glyph_indices;
            let has_images = !self.cells.drawn_image_runs.is_empty();
            if q_count > 0 || g_count > 0 || cg_count > 0 || has_images {
                pass.set_bind_group(0, &self.uniform_bg, &[]);
                if q_count > 0 {
                    pass.set_pipeline(&self.quad_pipeline);
                    pass.set_vertex_buffer(0, self.cells.quad_vb.slice(..));
                    pass.set_index_buffer(self.cells.quad_ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..q_count, 0, 0..1);
                }
                // Pictures sit between the shapes and the text, one draw per
                // run. That is a paint order, not a z-order: a shape the walk
                // drew after a picture still lands under it, the same way text
                // already lands over both. The board's real order is the
                // `paint-order rewrite` of `ambre-roadmap.md` E6.
                for (key, start, end) in &self.cells.drawn_image_runs {
                    // An entry evicted since the upload leaves its run undrawn
                    // rather than binding another picture's texture.
                    if let Some(bg) = self.images.bound(*key) {
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(1, bg, &[]);
                        pass.set_vertex_buffer(0, self.cells.image_vb.slice(..));
                        pass.set_index_buffer(
                            self.cells.image_ib.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(*start..*end, 0, 0..1);
                    }
                }
                if g_count > 0 {
                    pass.set_pipeline(&self.glyph_pipeline);
                    pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.cells.glyph_vb.slice(..));
                    pass.set_index_buffer(self.cells.glyph_ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..g_count, 0, 0..1);
                }
                if cg_count > 0 {
                    pass.set_pipeline(&self.color_glyph_pipeline);
                    pass.set_bind_group(1, &self.color_atlas_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.cells.color_glyph_vb.slice(..));
                    pass.set_index_buffer(
                        self.cells.color_glyph_ib.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..cg_count, 0, 0..1);
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
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
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
        let overlay_cg = self.overlay.drawn_color_glyph_indices;
        let overlay_img = !self.overlay.drawn_image_runs.is_empty();
        if overlay_q > 0 || overlay_g > 0 || overlay_cg > 0 || overlay_img {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: swapchain_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
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
            for (key, start, end) in &self.overlay.drawn_image_runs {
                if let Some(bg) = self.images.bound(*key) {
                    pass.set_pipeline(&self.image_pipeline);
                    pass.set_bind_group(1, bg, &[]);
                    pass.set_vertex_buffer(0, self.overlay.image_vb.slice(..));
                    pass.set_index_buffer(
                        self.overlay.image_ib.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(*start..*end, 0, 0..1);
                }
            }
            if overlay_g > 0 {
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                pass.set_vertex_buffer(0, self.overlay.glyph_vb.slice(..));
                pass.set_index_buffer(self.overlay.glyph_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..overlay_g, 0, 0..1);
            }
            if overlay_cg > 0 {
                pass.set_pipeline(&self.color_glyph_pipeline);
                pass.set_bind_group(1, &self.color_atlas_bind_group, &[]);
                pass.set_vertex_buffer(0, self.overlay.color_glyph_vb.slice(..));
                pass.set_index_buffer(
                    self.overlay.color_glyph_ib.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..overlay_cg, 0, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod image_cache_tests {
    use super::*;

    fn entry(bytes: usize, w: u32, h: u32, last_used: u64) -> CacheEntry<u32> {
        CacheEntry {
            payload: w,
            bytes,
            width: w,
            height: h,
            last_used,
        }
    }

    /// A resident picture is a hit, and the hit stamps the frame clock
    /// (`CANVAS-066`): without the stamp every entry looks equally old and
    /// eviction picks at random rather than least-recently-drawn.
    #[test]
    fn a_hit_stamps_the_entry_with_the_current_frame() {
        let mut c: ImageCache<u32> = ImageCache::new(1024);
        c.insert(ImageKey(1), entry(16, 2, 2, 0));
        c.clock = 7;
        assert_eq!(c.get(ImageKey(1), 2, 2), Some(&2));
        assert_eq!(c.map[&ImageKey(1)].last_used, 7);
    }

    /// A key that comes back at a different size is a miss, so the caller
    /// re-uploads instead of stretching the old texture over new content
    /// (`ImageSource::key`).
    #[test]
    fn a_key_at_another_size_is_a_miss() {
        let mut c: ImageCache<u32> = ImageCache::new(1024);
        c.insert(ImageKey(1), entry(16, 2, 2, 0));
        assert_eq!(c.get(ImageKey(1), 4, 4), None);
        assert!(c.get(ImageKey(1), 2, 2).is_some());
    }

    /// Over budget, the entry drawn longest ago goes first and the recent ones
    /// stay (`CANVAS-066`). A cache that evicted by insertion order would drop
    /// the picture the user is looking at to keep one they scrolled past.
    #[test]
    fn the_least_recently_drawn_entry_is_evicted_first() {
        let mut c: ImageCache<u32> = ImageCache::new(100);
        c.insert(ImageKey(1), entry(40, 1, 1, 5)); // drawn recently
        c.insert(ImageKey(2), entry(40, 2, 2, 1)); // drawn long ago
        assert_eq!(c.bytes, 80);
        c.insert(ImageKey(3), entry(40, 3, 3, 9));
        assert!(c.bound(ImageKey(2)).is_none(), "the stalest entry stays");
        assert!(c.bound(ImageKey(1)).is_some());
        assert!(c.bound(ImageKey(3)).is_some());
        assert_eq!(c.bytes, 80);
    }

    /// Re-inserting a key replaces it rather than counting its bytes twice: an
    /// image re-uploaded at a new size must not leak the budget of the old one.
    #[test]
    fn reinserting_a_key_replaces_its_cost() {
        let mut c: ImageCache<u32> = ImageCache::new(1024);
        c.insert(ImageKey(1), entry(40, 1, 1, 0));
        c.insert(ImageKey(1), entry(64, 1, 1, 0));
        assert_eq!(c.map.len(), 1);
        assert_eq!(c.bytes, 64);
    }

    /// One picture larger than the whole budget is kept: evicting down to
    /// nothing would paint a placeholder where the user put a photograph, and
    /// the next insert reclaims it anyway.
    #[test]
    fn a_lone_oversized_entry_survives() {
        let mut c: ImageCache<u32> = ImageCache::new(100);
        c.insert(ImageKey(1), entry(4096, 1, 1, 0));
        assert!(c.bound(ImageKey(1)).is_some());
        assert_eq!(c.map.len(), 1);
    }
}

#[cfg(test)]
mod image_run_tests {
    use super::*;

    fn runs_of(spec: &[(u64, u32, u32)]) -> Vec<(u64, u32, u32)> {
        let mut runs = Vec::new();
        for (k, s, e) in spec {
            extend_runs(&mut runs, ImageKey(*k), *s, *e);
        }
        runs.iter().map(|r| (r.key.0, r.start, r.end)).collect()
    }

    /// Consecutive quads of one picture collapse into a single draw
    /// (`CANVAS-066`): a board showing ten copies of one logo costs one bind and
    /// one `draw_indexed`, not ten.
    #[test]
    fn consecutive_quads_of_one_picture_are_one_run() {
        assert_eq!(
            runs_of(&[(7, 0, 6), (7, 6, 12), (7, 12, 18)]),
            vec![(7, 0, 18)]
        );
    }

    /// Two pictures keep the walk's order, and coming back to the first opens a
    /// third run rather than reopening the first. Merging across the second
    /// would draw the first picture's later quads underneath it.
    #[test]
    fn a_different_picture_opens_a_run_and_order_is_kept() {
        assert_eq!(
            runs_of(&[(1, 0, 6), (2, 6, 12), (1, 12, 18)]),
            vec![(1, 0, 6), (2, 6, 12), (1, 12, 18)]
        );
    }

    /// A gap in the index range opens a run even for the same picture: the
    /// merged draw would otherwise cover indices no quad of this run wrote.
    #[test]
    fn a_discontinuous_range_opens_a_run() {
        assert_eq!(
            runs_of(&[(1, 0, 6), (1, 12, 18)]),
            vec![(1, 0, 6), (1, 12, 18)]
        );
    }
}
