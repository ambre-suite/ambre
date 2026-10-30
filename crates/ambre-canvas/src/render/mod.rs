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
//   is rendered to a ping-pong pair of offscreen textures. Each frame picks one
//   of three plans. Full repaints everything. Scroll shifts the previous frame
//   via `copy_texture_to_texture` and redraws only the entering strip. None
//   skips the cells layer entirely and just re-blits. The OVERLAY layer
//   (headers, active border, HUD) is transient and re-rendered to the
//   swapchain every dirty frame.
//
// Public API:
//   QuadRenderer  -- owns all pipelines, GlyphAtlas, per-layer CPU batches and
//                    GPU buffers, plus the ping-pong offscreen pair.
//   WgpuSurface   -- frame-scoped DrawSurface. Holds the clip stack.
//                    Routes pushes to the renderer's currently active layer.
// ============================================================================
use crate::density::{DevicePx, DeviceSize};
use ambre_core::ViewPx;
use ambre_draw::{Color, ImageKey};
use bytemuck::{Pod, Zeroable};

// The renderer is split across sibling modules; `atlas`/`renderer`/`surface`
// reach the shared primitives below (vertex types, `LayerBatches`, helpers) via
// `use super::*`. Public API is re-exported to keep `render::*` paths stable.
mod atlas;
mod mipmap;
mod renderer;
mod surface;

pub use atlas::{AtlasKind, AtlasSlot, GlyphAtlas, GlyphId, RasterSize};
pub use renderer::{ContentRepaint, LayerRepaint, QuadRenderer};
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

/// One axis-aligned glyph quad in view pixels, and the box of the atlas it
/// samples. The lengths are unwrapped into the `f32` of `GlyphVertex` alone.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GlyphQuad {
    /// The left edge.
    x: ViewPx,
    /// The top edge.
    y: ViewPx,
    width: ViewPx,
    height: ViewPx,
    /// The texture coordinate of the top-left corner.
    uv_min: [f32; 2],
    /// The texture coordinate of the bottom-right corner.
    uv_max: [f32; 2],
}

const GLYPH_VERTEX_SIZE: usize = std::mem::size_of::<GlyphVertex>();
const INITIAL_GLYPHS: usize = 4096;

// ============================================================================
// Image pipeline
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageVertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

const IMAGE_VERTEX_SIZE: usize = std::mem::size_of::<ImageVertex>();

/// A stretch of the image index buffer drawn with one texture bound.
///
/// Images cannot collapse into a single draw the way quads and glyphs do. Each
/// picture is its own texture, so each needs its own bind group. What a run buys
/// instead is *order*. The batch is filled in the order the walk drew.
/// Consecutive quads sharing a texture extend the run they are already in. A
/// board of ten copies of one picture costs one draw, and two overlapping
/// pictures keep the walk's own order between them.
struct ImageRun {
    key: ImageKey,
    /// Index range into the layer's image index buffer.
    start: u32,
    end: u32,
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
/// each other. The `drawn_*` fields record the last uploaded counts. An idle
/// frame can then re-encode draw calls using the previously uploaded GPU
/// buffers, without rebuilding the CPU side.
struct LayerBatches {
    quad_vertices: Vec<QuadVertex>,
    quad_indices: Vec<u32>,
    glyph_vertices: Vec<GlyphVertex>,
    glyph_indices: Vec<u32>,
    /// Color-glyph batch (color emoji): same `GlyphVertex` layout, drawn by the
    /// color pipeline against the RGBA color atlas.
    color_glyph_vertices: Vec<GlyphVertex>,
    color_glyph_indices: Vec<u32>,
    /// Image batch: one vertex buffer for the layer, cut into `image_runs` by
    /// the texture each stretch needs bound.
    image_vertices: Vec<ImageVertex>,
    image_indices: Vec<u32>,
    image_runs: Vec<ImageRun>,
    quad_vb: wgpu::Buffer,
    quad_ib: wgpu::Buffer,
    glyph_vb: wgpu::Buffer,
    glyph_ib: wgpu::Buffer,
    color_glyph_vb: wgpu::Buffer,
    color_glyph_ib: wgpu::Buffer,
    image_vb: wgpu::Buffer,
    image_ib: wgpu::Buffer,
    /// Number of quad indices in the last upload. Used as the draw_indexed
    /// upper bound on any subsequent frame that reuses the GPU buffer.
    drawn_quad_indices: u32,
    drawn_glyph_indices: u32,
    drawn_color_glyph_indices: u32,
    /// The runs of the last upload, kept whole. An idle frame re-encodes the
    /// same draws against the same buffer, as the `drawn_*` counts do for the
    /// other three batches.
    drawn_image_runs: Vec<(ImageKey, u32, u32)>,
    /// Armed by `begin_rebuild()`. When armed, `upload_if_rebuilding()`
    /// transfers the CPU vecs to the GPU and refreshes `drawn_*`. Otherwise
    /// the GPU buffer state is preserved across frames.
    rebuilding: RebuildLatch,
}

/// Armed when a layer's CPU batch is rebuilt, taken by its upload.
///
/// A layer the frame never begins is never armed, so a `Reuse` plan uploads
/// nothing (`CANVAS-003`). One rebuild uploads once.
#[derive(Default)]
struct RebuildLatch(bool);

impl RebuildLatch {
    /// The batch is being rebuilt this frame.
    fn arm(&mut self) {
        self.0 = true;
    }

    /// Whether an upload is owed, disarming the latch.
    fn take(&mut self) -> bool {
        std::mem::take(&mut self.0)
    }
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
        // Images are few per frame even on a busy board: start small and grow.
        let image_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_image_vb")),
            size: (64 * 4 * IMAGE_VERTEX_SIZE) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let image_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_image_ib")),
            size: (64 * 6 * 4) as u64,
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
            image_vertices: Vec::new(),
            image_indices: Vec::new(),
            image_runs: Vec::new(),
            quad_vb,
            quad_ib,
            glyph_vb,
            glyph_ib,
            color_glyph_vb,
            color_glyph_ib,
            image_vb,
            image_ib,
            drawn_quad_indices: 0,
            drawn_glyph_indices: 0,
            drawn_color_glyph_indices: 0,
            drawn_image_runs: Vec::new(),
            rebuilding: RebuildLatch::default(),
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
        self.image_vertices.clear();
        self.image_indices.clear();
        self.image_runs.clear();
        self.rebuilding.arm();
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
        if !self.rebuilding.take() {
            return;
        }
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
        if !self.image_indices.is_empty() {
            grow_buffer_if_needed(
                device,
                &mut self.image_vb,
                self.image_vertices.len() * IMAGE_VERTEX_SIZE,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "image_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.image_ib,
                self.image_indices.len() * 4,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                "image_ib",
                label_prefix,
            );
            queue.write_buffer(
                &self.image_vb,
                0,
                bytemuck::cast_slice(&self.image_vertices),
            );
            queue.write_buffer(&self.image_ib, 0, bytemuck::cast_slice(&self.image_indices));
        }
        // Refresh the drawn counts for subsequent encode passes.
        self.drawn_image_runs.clear();
        self.drawn_image_runs
            .extend(self.image_runs.iter().map(|r| (r.key, r.start, r.end)));
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

/// The clear colour of a pass, in the same components a vertex carries, so a
/// cleared pixel and a filled one agree (`CANVAS-060`).
fn clear_color(c: Color) -> wgpu::Color {
    let [r, g, b, a] = color_f32(c).map(f64::from);
    wgpu::Color { r, g, b, a }
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

/// Encode a `copy_texture_to_texture` that shifts `src` by `(dx, dy)`
/// pixels into `dst`. Negative deltas shift content left/up; positive shift
/// right/down. The strip exposed by the shift is *not* cleared -- the caller
/// must render fresh geometry over it in the subsequent cells pass.
///
/// WebGL2 forbids same-texture src/dst, hence the ping-pong pair: `src` and
/// `dst` must be distinct GPU textures.
fn copy_offscreen_with_offset(
    encoder: &mut wgpu::CommandEncoder,
    src: &wgpu::Texture,
    dst: &wgpu::Texture,
    size: DeviceSize,
    dx: DevicePx,
    dy: DevicePx,
) {
    // wgpu takes the bare edges.
    let (width, height) = (size.width.get(), size.height.get());
    // Compute the overlapping region: source rect that has a valid mapping
    // into the destination. Out-of-range deltas leave nothing to copy.
    let dx_abs = dx.unsigned_abs();
    let dy_abs = dy.unsigned_abs();
    if dx_abs >= width || dy_abs >= height {
        return;
    }
    let copy_w = width - dx_abs;
    let copy_h = height - dy_abs;

    let (src_x, dst_x) = if dx >= DevicePx::ZERO {
        // Content shifts right: take src starting at x=0, place at dst x=dx.
        (0u32, dx_abs)
    } else {
        // Content shifts left: take src starting at x=dx_abs, place at dst x=0.
        (dx_abs, 0u32)
    };
    let (src_y, dst_y) = if dy >= DevicePx::ZERO {
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

#[cfg(test)]
mod tests {
    use super::{RebuildLatch, clear_color, color_f32};

    /// A layer the frame never began uploads nothing, and a rebuilt one
    /// uploads once (`CANVAS-003`).
    ///
    /// A skipped upload that still ran would cost frame time and look correct
    /// on screen.
    #[test]
    fn a_layer_uploads_only_on_the_frame_that_rebuilt_it() {
        let mut latch = RebuildLatch::default();
        assert!(!latch.take(), "a reused layer owes no upload");
        latch.arm();
        assert!(latch.take(), "a rebuilt layer uploads");
        assert!(!latch.take(), "and only once");
    }

    /// Covers `CANVAS-060`: a full rebuild clears to the colour the view
    /// handed over, in the components its fills carry.
    #[test]
    fn a_pass_clears_to_the_palette_colour() {
        for theme in [crate::Theme::Light, crate::Theme::Dark] {
            let bg = theme.palette().content_bg;
            let clear = clear_color(bg);
            let [r, g, b, a] = color_f32(bg).map(f64::from);
            assert_eq!((clear.r, clear.g, clear.b, clear.a), (r, g, b, a));
        }
        assert_ne!(
            clear_color(crate::Theme::Dark.palette().content_bg),
            wgpu::Color::WHITE,
            "a dark frame no longer clears to white"
        );
    }
}
