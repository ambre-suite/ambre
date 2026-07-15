// ambre-canvas/src/render/mod.rs
// ============================================================================
// CPU-batched renderer backed by wgpu vertex/fragment shaders.
//
// No compute shaders -- every draw call maps to plain triangles + textured quads.
//
// Four pipelines, flushed in order each frame:
//   1. Quad pipeline        -- solid-color filled/stroked rectangles and lines.
//   2. Glyph pipeline       -- alpha-mask glyphs from the GlyphAtlas (R8Unorm).
//   3. Color glyph pipeline -- full-color glyphs (emoji) from the RGBA color
//                              atlas (Rgba8Unorm), sampled untinted.
//   4. Blit pipeline        -- samples the persistent cells offscreen onto swapchain.
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
use ambre_draw::Color;
use bytemuck::{Pod, Zeroable};

// The renderer is split across sibling modules; `atlas`/`renderer`/`surface`
// reach the shared primitives below (vertex types, `LayerBatches`, helpers) via
// `use super::*`. Public API is re-exported to keep `render::*` paths stable.
mod atlas;
mod renderer;
mod surface;

pub use atlas::{AtlasSlot, GlyphAtlas};
pub use renderer::{ContentRepaint, OverlayRepaint, QuadRenderer};
pub use surface::WgpuSurface;
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
    quad_indices: Vec<u32>,
    glyph_vertices: Vec<GlyphVertex>,
    glyph_indices: Vec<u32>,
    /// Color-glyph batch (color emoji): same `GlyphVertex` layout, drawn by the
    /// color pipeline against the RGBA color atlas.
    color_glyph_vertices: Vec<GlyphVertex>,
    color_glyph_indices: Vec<u32>,
    quad_vb: wgpu::Buffer,
    quad_ib: wgpu::Buffer,
    glyph_vb: wgpu::Buffer,
    glyph_ib: wgpu::Buffer,
    color_glyph_vb: wgpu::Buffer,
    color_glyph_ib: wgpu::Buffer,
    /// Number of quad indices in the last upload. Used as the draw_indexed
    /// upper bound on any subsequent frame that reuses the GPU buffer.
    drawn_quad_indices: u32,
    drawn_glyph_indices: u32,
    drawn_color_glyph_indices: u32,
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
        // Color glyphs are rare (emoji): start small, grow on demand.
        let color_glyph_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_color_glyph_vb")),
            size: (256 * 4 * GLYPH_VERTEX_SIZE) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let color_glyph_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_color_glyph_ib")),
            size: (256 * 6 * 4) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            quad_vertices: Vec::with_capacity(INITIAL_QUADS * 4),
            quad_indices: Vec::with_capacity(INITIAL_QUADS * 6),
            glyph_vertices: Vec::with_capacity(INITIAL_GLYPHS * 4),
            glyph_indices: Vec::with_capacity(INITIAL_GLYPHS * 6),
            color_glyph_vertices: Vec::new(),
            color_glyph_indices: Vec::new(),
            quad_vb,
            quad_ib,
            glyph_vb,
            glyph_ib,
            color_glyph_vb,
            color_glyph_ib,
            drawn_quad_indices: 0,
            drawn_glyph_indices: 0,
            drawn_color_glyph_indices: 0,
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
        self.color_glyph_vertices.clear();
        self.color_glyph_indices.clear();
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
        if !self.color_glyph_indices.is_empty() {
            grow_buffer_if_needed(
                device,
                &mut self.color_glyph_vb,
                self.color_glyph_vertices.len() * GLYPH_VERTEX_SIZE,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "color_glyph_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.color_glyph_ib,
                self.color_glyph_indices.len() * 4,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                "color_glyph_ib",
                label_prefix,
            );
            queue.write_buffer(
                &self.color_glyph_vb,
                0,
                bytemuck::cast_slice(&self.color_glyph_vertices),
            );
            queue.write_buffer(
                &self.color_glyph_ib,
                0,
                bytemuck::cast_slice(&self.color_glyph_indices),
            );
        }
        // Refresh the drawn counts for subsequent encode passes.
        self.drawn_quad_indices = self.quad_indices.len() as u32;
        self.drawn_glyph_indices = self.glyph_indices.len() as u32;
        self.drawn_color_glyph_indices = self.color_glyph_indices.len() as u32;
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
    width: u32,
    height: u32,
    dx_px: i32,
    dy_px: i32,
) {
    // Compute the overlapping region: source rect that has a valid mapping
    // into the destination. Out-of-range deltas leave nothing to copy.
    let dx_abs = dx_px.unsigned_abs();
    let dy_abs = dy_px.unsigned_abs();
    if dx_abs >= width || dy_abs >= height {
        return;
    }
    let copy_w = width - dx_abs;
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
            origin: wgpu::Origin3d {
                x: src_x,
                y: src_y,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: dst_x,
                y: dst_y,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: copy_w,
            height: copy_h,
            depth_or_array_layers: 1,
        },
    );
}
