// ambre-canvas/src/render/renderer.rs
// ============================================================================
// QuadRenderer: owns the three wgpu pipelines (quad, glyph, blit), the
// GlyphAtlas, the ping-pong offscreen pair for the persistent cells layer, and
// the per-frame dirty-rect flush. Also the offscreen target and the repaint
// plan enums. The `include_str!("*.wgsl")` calls resolve against this dir.
// ============================================================================

use super::*;
use ambre_draw::{Color, Rect};

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
    blit_pipeline: wgpu::RenderPipeline,

    // -- Shared uniform (viewport size) --
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,

    // -- Glyph atlas + its bind group --
    atlas_bind_group: wgpu::BindGroup,
    pub atlas: GlyphAtlas,

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
        if overlay_q > 0 || overlay_g > 0 {
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
