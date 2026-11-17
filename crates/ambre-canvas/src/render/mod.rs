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
use ambre_core::VideoSlots;
use ambre_core::index::{Axis, Len, Pos, Span};
use ambre_core::{Count, Device, NarrowCount, Octets, Point, Px, Size, Texel, View};
use ambre_draw::{Color, ImageKey};
use bytemuck::{Pod, Zeroable};

use crate::space::Uv;

// The renderer is split across sibling modules; `atlas`/`renderer`/`surface`
// reach the shared primitives below (vertex types, `LayerBatches`, helpers) via
// `use super::*`. Public API is re-exported to keep `render::*` paths stable.
mod atlas;
mod mipmap;
mod renderer;
mod surface;

pub(crate) use atlas::raster_of_layout;
pub use atlas::{AtlasKind, AtlasSlot, GlyphAtlas, GlyphId, UvBox};
pub use renderer::{ContentRepaint, LayerRepaint, QuadRenderer, VideosDrawn};
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

impl QuadVertex {
    /// A vertex at `pos`, filled with `color`.
    fn new(pos: Point<Px<View>>, color: Color) -> Self {
        Self {
            pos: [pos.x.get(), pos.y.get()],
            // peniko::AlphaColor<Srgb> stores [r, g, b, a] as f32 0..=1.
            color: color.components,
        }
    }
}

/// The quads a layer's quad buffers hold before they first grow.
const INITIAL_QUADS: Count<Quads> = Count::new(8192);

/// The quads of a batch: a rectangle, a line, a glyph or a picture, each
/// four vertices and six indices.
enum Quads {}

impl NarrowCount for Quads {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Quads");
}

/// The bytes `quads` quads of the vertex `V` take: four vertices each.
fn vertex_bytes<V>(quads: Count<Quads>) -> Count<Octets> {
    Count::<Octets>::new(u64::from(quads.get()) * 4 * std::mem::size_of::<V>() as u64)
}

/// The bytes the indices of `quads` quads take: six `u32` each.
fn index_bytes(quads: Count<Quads>) -> Count<Octets> {
    Count::<Octets>::new(u64::from(quads.get()) * 6 * std::mem::size_of::<u32>() as u64)
}

/// The slots of an index buffer: where a draw starts and stops reading it.
#[derive(Debug, PartialEq, Eq)]
enum IndexSlots {}

impl Axis for IndexSlots {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("IndexSlot");
}

/// The len of an index list a batch holds, as a draw reads it. A batch never
/// holds four billion indices: the buffer could not be allocated first.
fn indices_len<I>(indices: &[I]) -> Len<IndexSlots> {
    Len::new(u32::try_from(indices.len()).unwrap_or(u32::MAX))
}

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

impl GlyphVertex {
    /// A vertex at `pos` that samples the atlas at `uv`, tinted `tint`. A
    /// colour glyph has no tint: its pipeline ignores the opaque black it
    /// carries.
    fn new(pos: Point<Px<View>>, uv: Point<Px<Uv>>, tint: Option<Color>) -> Self {
        Self {
            pos: [pos.x.get(), pos.y.get()],
            uv: [uv.x.get(), uv.y.get()],
            color: tint.map_or([0.0, 0.0, 0.0, 1.0], |c| c.components),
        }
    }
}

/// One axis-aligned glyph quad in view pixels, and the box of the atlas it
/// samples. The lengths are unwrapped into the `f32` of `GlyphVertex` alone.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GlyphQuad {
    /// The left edge.
    x: Px<View>,
    /// The top edge.
    y: Px<View>,
    width: Px<View>,
    height: Px<View>,
    /// The box of the atlas the quad samples.
    uv: UvBox,
}

/// The glyph quads a layer's glyph buffers hold before they first grow.
const INITIAL_GLYPHS: Count<Quads> = Count::new(4096);

// ============================================================================
// Image pipeline
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageVertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

impl ImageVertex {
    /// A vertex at `pos` that samples its picture at `uv`.
    fn new(pos: Point<Px<View>>, uv: Point<Px<Uv>>) -> Self {
        Self {
            pos: [pos.x.get(), pos.y.get()],
            uv: [uv.x.get(), uv.y.get()],
        }
    }
}

/// A stretch of the image index buffer drawn with one texture bound.
///
/// Images cannot collapse into a single draw the way quads and glyphs do. Each
/// picture is its own texture, so each needs its own bind group. What a run buys
/// instead is *order*. The batch is filled in the order the walk drew.
/// Consecutive quads sharing a texture extend the run they are already in. A
/// board of ten copies of one picture costs one draw, and two overlapping
/// pictures keep the walk's own order between them.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ImageRun {
    key: Binding,
    /// Index range into the layer's image index buffer.
    indices: Span<IndexSlots>,
}
/// The texture one run of the image batch binds. It holds a picture the caller decoded, or the
/// picture of a received video, copied at this paint (`CALL-015`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Binding {
    /// A decoded picture, by the key its caller gave it.
    Picture(ImageKey),
    /// The video of a call, by its slot.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(dead_code, reason = "the page alone holds a video")
    )]
    Video(Pos<VideoSlots>),
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
    drawn_quad_indices: Len<IndexSlots>,
    drawn_glyph_indices: Len<IndexSlots>,
    drawn_color_glyph_indices: Len<IndexSlots>,
    /// The runs of the last upload, kept whole. An idle frame re-encodes the
    /// same draws against the same buffer, as the `drawn_*` counts do for the
    /// other three batches.
    drawn_image_runs: Vec<ImageRun>,
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
            size: vertex_bytes::<QuadVertex>(INITIAL_QUADS).get(),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_quad_ib")),
            size: index_bytes(INITIAL_QUADS).get(),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_glyph_vb")),
            size: vertex_bytes::<GlyphVertex>(INITIAL_GLYPHS).get(),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_glyph_ib")),
            size: index_bytes(INITIAL_GLYPHS).get(),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Color glyphs are rare (emoji): start small, grow on demand.
        let color_glyph_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_color_glyph_vb")),
            size: vertex_bytes::<GlyphVertex>(Count::new(256)).get(),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let color_glyph_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_color_glyph_ib")),
            size: index_bytes(Count::new(256)).get(),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Images are few per frame even on a busy board: start small and grow.
        let image_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_image_vb")),
            size: vertex_bytes::<ImageVertex>(Count::new(64)).get(),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let image_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_image_ib")),
            size: index_bytes(Count::new(64)).get(),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            quad_vertices: Vec::with_capacity(INITIAL_QUADS.get() as usize * 4),
            quad_indices: Vec::with_capacity(INITIAL_QUADS.get() as usize * 6),
            glyph_vertices: Vec::with_capacity(INITIAL_GLYPHS.get() as usize * 4),
            glyph_indices: Vec::with_capacity(INITIAL_GLYPHS.get() as usize * 6),
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
            drawn_quad_indices: Len::ZERO,
            drawn_glyph_indices: Len::ZERO,
            drawn_color_glyph_indices: Len::ZERO,
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
                Count::<Octets>::of(std::mem::size_of_val(self.quad_vertices.as_slice())),
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "quad_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.quad_ib,
                Count::<Octets>::of(std::mem::size_of_val(self.quad_indices.as_slice())),
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
                Count::<Octets>::of(std::mem::size_of_val(self.glyph_vertices.as_slice())),
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "glyph_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.glyph_ib,
                Count::<Octets>::of(std::mem::size_of_val(self.glyph_indices.as_slice())),
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
                Count::<Octets>::of(std::mem::size_of_val(self.color_glyph_vertices.as_slice())),
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "color_glyph_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.color_glyph_ib,
                Count::<Octets>::of(std::mem::size_of_val(self.color_glyph_indices.as_slice())),
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
                Count::<Octets>::of(std::mem::size_of_val(self.image_vertices.as_slice())),
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "image_vb",
                label_prefix,
            );
            grow_buffer_if_needed(
                device,
                &mut self.image_ib,
                Count::<Octets>::of(std::mem::size_of_val(self.image_indices.as_slice())),
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
        self.drawn_image_runs.extend_from_slice(&self.image_runs);
        self.drawn_quad_indices = indices_len(&self.quad_indices);
        self.drawn_glyph_indices = indices_len(&self.glyph_indices);
        self.drawn_color_glyph_indices = indices_len(&self.color_glyph_indices);
    }
}
// ============================================================================
// Helpers
// ============================================================================

/// The clear colour of a pass, in the same components a vertex carries, so a
/// cleared pixel and a filled one agree (`CANVAS-060`).
fn clear_color(c: Color) -> wgpu::Color {
    // peniko::AlphaColor<Srgb> stores [r, g, b, a] as f32 0..=1.
    let [r, g, b, a] = c.components.map(f64::from);
    wgpu::Color { r, g, b, a }
}

/// Recreate `buf` if `needed_bytes` exceeds its current size.
/// Allocates at 2x the needed size to amortize future resizes.
fn grow_buffer_if_needed(
    device: &wgpu::Device,
    buf: &mut wgpu::Buffer,
    needed_bytes: Count<Octets>,
    usage: wgpu::BufferUsages,
    name: &'static str,
    label_prefix: &str,
) {
    if needed_bytes.get() > buf.size() {
        let new_size = needed_bytes.plus(needed_bytes).get().next_power_of_two();
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
    size: Size<Px<Texel>>,
    dx: Px<Device>,
    dy: Px<Device>,
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

    let (src_x, dst_x) = if dx >= Px::<Device>::ZERO {
        // Content shifts right: take src starting at x=0, place at dst x=dx.
        (0u32, dx_abs)
    } else {
        // Content shifts left: take src starting at x=dx_abs, place at dst x=0.
        (dx_abs, 0u32)
    };
    let (src_y, dst_y) = if dy >= Px::<Device>::ZERO {
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
    use super::{RebuildLatch, clear_color};

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
            let [r, g, b, a] = bg.components.map(f64::from);
            assert_eq!((clear.r, clear.g, clear.b, clear.a), (r, g, b, a));
        }
        assert_ne!(
            clear_color(crate::Theme::Dark.palette().content_bg),
            wgpu::Color::WHITE,
            "a dark frame no longer clears to white"
        );
    }
}
