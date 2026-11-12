// ambre-canvas/src/render/surface.rs
// ============================================================================
// WgpuSurface: the frame-scoped `DrawSurface` adapter over `QuadRenderer`. Owns
// the clip stack and routes fill/stroke/glyph pushes to the renderer active
// layer, with per-glyph atlas rasterization and clip trimming.
// ============================================================================

use super::*;
use ambre_core::{Css, Device, Px, Scale, View};
use ambre_draw::{
    Affine, Brush, Color, DrawSurface, GlyphRun, ImageSource, Line, ParleyBrush, Point, Rect,
    Stroke,
};
use kurbo::{BezPath, PathEl, Shape};
use lyon::math::point as lpoint;
use lyon::path::{LineCap, LineJoin, Path as LyonPath};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};

// Flattening tolerance in *device* pixels. Divided by the active transform's
// scale and by the density before tessellation, so a zoomed-in Toile path and
// a dense screen both stay smooth. We tessellate in model space, then map
// vertices to screen space.
const PATH_TOLERANCE_PX: f64 = 0.15;

/// A transform's linear part split into its scale and the rest. The scale is
/// the square root of the determinant, as a path's tolerance reads it. The
/// rest is the linear part divided by it.
///
/// The raster already carries the scale, so a rotated glyph quad is mapped by
/// the rest alone. `None` for a transform that collapses the plane.
fn split_linear(t: Affine) -> Option<(f64, Affine)> {
    let scale = affine_scale(t);
    if !(scale.is_finite() && scale > 1e-9) {
        return None;
    }
    let c = t.as_coeffs();
    let unit = Affine::new([
        c[0] / scale,
        c[1] / scale,
        c[2] / scale,
        c[3] / scale,
        0.0,
        0.0,
    ]);
    Some((scale, unit))
}

/// A glyph bitmap's box in view pixels, the unit the quad takes. The four
/// are lengths of one unit side by side, so each is named.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GlyphBox {
    /// From the pen to the bitmap's left edge.
    left_bearing: Px<View>,
    /// From the baseline up to the bitmap's top edge.
    top_bearing: Px<View>,
    width: Px<View>,
    height: Px<View>,
}

/// A glyph bitmap's logical box. A texel covers `texel_scale` device pixels,
/// one below the raster cap (`CANVAS-087`). So each length is scaled by it and
/// divided by the density, and the glyph covers the same logical area on
/// every screen.
fn glyph_box(slot: &AtlasSlot, density: Scale<Css, Device>) -> GlyphBox {
    let per_texel = slot.texel_scale / density.get() as f32;
    GlyphBox {
        left_bearing: Px::<View>::new(slot.bearing_left as f32 * per_texel),
        top_bearing: Px::<View>::new(slot.bearing_top as f32 * per_texel),
        width: Px::<View>::new(slot.pixel_width as f32 * per_texel),
        height: Px::<View>::new(slot.pixel_height as f32 * per_texel),
    }
}

/// The two batches a placed glyph lands in.
///
/// A `tint` of `Some` names the alpha atlas, drawn in that colour. `None` names
/// the colour atlas, drawn untinted. [`QuadRenderer`] is the frame's sink.
pub(super) trait GlyphSink {
    /// An axis-aligned quad, in screen pixels.
    fn glyph(&mut self, quad: GlyphQuad, tint: Option<Color>);
    /// A rotated quad, by its four screen corners, showing the atlas texels
    /// of `slot`.
    fn glyph_corners(
        &mut self,
        corners: [ambre_core::Point<Px<View>>; 4],
        slot: &AtlasSlot,
        tint: Option<Color>,
    );
}

impl GlyphSink for QuadRenderer {
    fn glyph(&mut self, quad: GlyphQuad, tint: Option<Color>) {
        match tint {
            Some(color) => self.push_glyph(quad, color),
            None => self.push_color_glyph(quad),
        }
    }

    fn glyph_corners(
        &mut self,
        corners: [ambre_core::Point<Px<View>>; 4],
        slot: &AtlasSlot,
        tint: Option<Color>,
    ) {
        let corners = corners.map(|p| [p.x.get(), p.y.get()]);
        match tint {
            Some(color) => self.push_glyph_quad(corners, slot.uv_min, slot.uv_max, color),
            None => self.push_color_glyph_quad(corners, slot.uv_min, slot.uv_max),
        }
    }
}

/// Where one glyph of a run is drawn.
struct GlyphPlacement {
    /// The glyph origin on the baseline, in screen space.
    origin: Point,
    /// The run's linear part when it is rotated; `None` draws axis-aligned.
    linear: Option<Affine>,
    /// Device pixels per logical pixel.
    density: Scale<Css, Device>,
    /// The active clip, which trims an axis-aligned glyph.
    clip: Option<Rect>,
    /// The run's colour, which tints a mask glyph.
    color: Color,
}

/// Place one rasterized glyph and hand it to the batch its atlas names.
///
/// A colour glyph goes to the colour batch untinted; a mask glyph goes to the
/// alpha batch in the run's colour (`CANVAS-019`). An axis-aligned glyph
/// straddling the clip is trimmed, its UVs remapped, never drawn whole.
fn emit_glyph(sink: &mut impl GlyphSink, slot: &AtlasSlot, at: &GlyphPlacement) {
    let tint = match slot.atlas {
        AtlasKind::Mask => Some(at.color),
        AtlasKind::Color => None,
    };
    let GlyphBox {
        left_bearing: bl,
        top_bearing: bt,
        width: sw,
        height: sh,
    } = glyph_box(slot, at.density);
    let origin = at.origin;

    // Rotated run: corners = origin + linear * local offset. A rotated run
    // carries no cell clip, so it skips the trim below.
    if let Some(linear) = at.linear {
        let (bl, bt, w, h) = (
            f64::from(bl.get()),
            f64::from(bt.get()),
            f64::from(sw.get()),
            f64::from(sh.get()),
        );
        let local = [(bl, -bt), (bl + w, -bt), (bl + w, -bt + h), (bl, -bt + h)];
        let corners = local.map(|(ox, oy)| {
            let v = linear * Point::new(ox, oy);
            ambre_core::Point::new(
                Px::<View>::new((origin.x + v.x) as f32),
                Px::<View>::new((origin.y + v.y) as f32),
            )
        });
        sink.glyph_corners(corners, slot, tint);
        return;
    }

    // Bitmap top-left in screen space. The top bearing points up, so subtract.
    let (x0, y0) = (
        Px::<View>::new(origin.x as f32) + bl,
        Px::<View>::new(origin.y as f32) - bt,
    );
    let (x1, y1) = (x0 + sw, y0 + sh);
    let Some(clip) = at.clip else {
        let quad = GlyphQuad {
            x: x0,
            y: y0,
            width: sw,
            height: sh,
            uv_min: slot.uv_min,
            uv_max: slot.uv_max,
        };
        sink.glyph(quad, tint);
        return;
    };
    // A glyph straddling the clip is trimmed and its UVs remapped. Drawn whole,
    // an overflowing cell's last glyph spills into its neighbour.
    let (cx0, cy0, cx1, cy1) = (
        Px::<View>::new(clip.x0 as f32),
        Px::<View>::new(clip.y0 as f32),
        Px::<View>::new(clip.x1 as f32),
        Px::<View>::new(clip.y1 as f32),
    );
    if x1 <= cx0 || x0 >= cx1 || y1 <= cy0 || y0 >= cy1 {
        return;
    }
    let (nx0, nx1) = (x0.max(cx0), x1.min(cx1));
    let (ny0, ny1) = (y0.max(cy0), y1.min(cy1));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let u0 = lerp(slot.uv_min[0], slot.uv_max[0], (nx0 - x0) / sw);
    let u1 = lerp(slot.uv_min[0], slot.uv_max[0], (nx1 - x0) / sw);
    let v0 = lerp(slot.uv_min[1], slot.uv_max[1], (ny0 - y0) / sh);
    let v1 = lerp(slot.uv_min[1], slot.uv_max[1], (ny1 - y0) / sh);
    let quad = GlyphQuad {
        x: nx0,
        y: ny0,
        width: nx1 - nx0,
        height: ny1 - ny0,
        uv_min: [u0, v0],
        uv_max: [u1, v1],
    };
    sink.glyph(quad, tint);
}

// ============================================================================
// WgpuSurface
// ============================================================================

/// Frame-scoped `DrawSurface` backed by `QuadRenderer`. Push routing follows
/// `renderer.active_layer`, set by the caller before each phase.
pub struct WgpuSurface<'r> {
    renderer: &'r mut QuadRenderer,
    // Held for `draw_image` alone: a picture is uploaded to a texture the first
    // frame it is drawn, and every other primitive here only appends vertices.
    device: &'r wgpu::Device,
    queue: &'r wgpu::Queue,
    clip_stack: Vec<Rect>,
    // Composed model-to-screen transforms, one entry per live `push_transform`.
    // The top is the transform currently applied to incoming geometry; empty
    // means identity. Clip rects are transformed at push time, so the stored
    // clip stack is always in screen space and needs no transform on read.
    transform_stack: Vec<Affine>,
}

impl<'r> WgpuSurface<'r> {
    pub fn new(
        renderer: &'r mut QuadRenderer,
        device: &'r wgpu::Device,
        queue: &'r wgpu::Queue,
    ) -> Self {
        Self {
            renderer,
            device,
            queue,
            clip_stack: Vec::new(),
            transform_stack: Vec::new(),
        }
    }

    /// Switch the surface (and underlying renderer) to a different layer,
    /// arming it for rebuild this frame. Clears the clip and transform stacks
    /// since neither crosses layer boundaries.
    pub fn begin_layer(&mut self, layer: Layer) {
        self.renderer.begin_layer(layer);
        self.clip_stack.clear();
        self.transform_stack.clear();
    }

    /// Paint the current picture of the received video of `slot` into the box `dest`, its
    /// aspect kept (`ambre-spec-call-media.md section 3`, video on the canvas). The page hands
    /// the render runtime the video of each slot, so the caller names the slot alone and never
    /// holds a page element (`CALL-015`). The paint copies the picture into a GPU texture of the
    /// slot, then draws a textured quad. A slot that holds no picture yet draws nothing.
    ///
    /// A video is screen state, so it is no primitive of `DrawSurface`: a page export draws
    /// none (`CANVAS-056`).
    pub fn draw_video(&mut self, slot: Pos<VideoSlots>, dest: Rect) {
        // The page holds the element of each slot, so a host build draws no video.
        #[cfg(target_arch = "wasm32")]
        {
            let Some((element, size)) = crate::video::frame(slot) else {
                return;
            };
            let Some(corners) =
                crate::video::contain(size, dest).and_then(|fit| self.quad_corners(fit))
            else {
                return;
            };
            self.renderer
                .push_video(self.device, self.queue, slot, (&element, size), corners);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (slot, dest);
    }

    /// The four corners of `dest` on screen, `[tl, tr, br, bl]`, mapped through the active
    /// transform. `None` when `dest` holds no area, or when the clip rejects its box.
    fn quad_corners(&self, dest: Rect) -> Option<[[f32; 2]; 4]> {
        if dest.width() <= 0.0 || dest.height() <= 0.0 {
            return None;
        }
        let t = self.transform();
        // The four destination corners mapped individually, so a rotated element
        // carries its picture round with it. Mapping the rect's bounding box
        // instead -- what `fill_rect` does, correctly, for an axis-aligned
        // colour -- would leave every picture upright inside a growing box.
        let corners_world = [
            (dest.x0, dest.y0),
            (dest.x1, dest.y0),
            (dest.x1, dest.y1),
            (dest.x0, dest.y1),
        ];
        let mapped: [Point; 4] = corners_world.map(|(x, y)| t * Point::new(x, y));
        // Clip is a bounding-box reject here, as it is for a path. The quad is
        // textured, and trimming it would need the uv range trimmed with it
        // (`ambre-spec-drawing-surface.md section 5`).
        if let Some(c) = self.active_clip() {
            let (xs, ys): (Vec<f64>, Vec<f64>) = mapped.iter().map(|p| (p.x, p.y)).unzip();
            let bbox = Rect::new(
                xs.iter().cloned().fold(f64::INFINITY, f64::min),
                ys.iter().cloned().fold(f64::INFINITY, f64::min),
                xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            );
            if bbox.intersect(c).is_zero_area() {
                return None;
            }
        }
        Some(mapped.map(|p| [p.x as f32, p.y as f32]))
    }

    /// The active model-to-screen transform (identity when the stack is empty).
    fn transform(&self) -> Affine {
        self.transform_stack
            .last()
            .copied()
            .unwrap_or(Affine::IDENTITY)
    }

    /// The flattening tolerance in model units under `t`.
    fn tolerance(&self, t: Affine) -> f32 {
        // The ratio narrowed to f32 as a glyph reads it, as it always was.
        let scale = affine_scale(t) * f64::from(self.renderer.density().get() as f32);
        (PATH_TOLERANCE_PX / scale.max(1e-6)) as f32
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

    /// True when the path's screen-space bounding box does not intersect the
    /// active clip -- a cheap reject that skips tessellation entirely. Path
    /// meshes are not trimmed to the clip (arbitrary geometry), so this is the
    /// only clip interaction for `fill_path`/`stroke_path`.
    fn path_outside_clip(&self, path: &BezPath, t: Affine) -> bool {
        self.path_outside_clip_padded(path, t, 0.0)
    }

    /// As `path_outside_clip`, but inflates the path bbox by `pad` model units
    /// (half the stroke width) before the transform. A thick stroke is then
    /// not culled when its centerline sits just outside the clip.
    fn path_outside_clip_padded(&self, path: &BezPath, t: Affine, pad: f64) -> bool {
        let Some(clip) = self.active_clip() else {
            return false;
        };
        let bbox = path.bounding_box().inflate(pad, pad);
        let hit = clip.intersect(t.transform_rect_bbox(bbox));
        hit.width() <= 0.0 || hit.height() <= 0.0
    }
}

impl DrawSurface for WgpuSurface<'_> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let rect = self.transform().transform_rect_bbox(rect);
        let r = self.clip_rect(rect);
        self.renderer.push_quad(r, color);
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: Px<ambre_core::Surface>) {
        let width = width.get();
        let hw = width * 0.5;
        // Picture-frame decomposition: two full-width horizontal bars (which own
        // the four corners) plus two verticals INSET to sit strictly between
        // them. The quads share only zero-area edges, so no pixel is painted
        // twice. A translucent stroke keeps a uniform alpha instead of the
        // doubled corners a naive 4-overlapping-quad frame produces.
        self.fill_rect(
            Rect::new(rect.x0 - hw, rect.y0 - hw, rect.x1 + hw, rect.y0 + hw),
            color,
        );
        self.fill_rect(
            Rect::new(rect.x0 - hw, rect.y1 - hw, rect.x1 + hw, rect.y1 + hw),
            color,
        );
        // Skip the inset verticals when the rect is shorter than the stroke.
        // The two horizontal bars already overlap and cover the whole span. An
        // inverted `y0 + hw > y1 - hw` rect would otherwise produce garbage.
        if rect.y1 - rect.y0 > width {
            self.fill_rect(
                Rect::new(rect.x0 - hw, rect.y0 + hw, rect.x0 + hw, rect.y1 - hw),
                color,
            );
            self.fill_rect(
                Rect::new(rect.x1 - hw, rect.y0 + hw, rect.x1 + hw, rect.y1 - hw),
                color,
            );
        }
    }

    fn stroke_line(&mut self, line: Line, color: Color, width: Px<ambre_core::Surface>) {
        let width = width.get();
        let t = self.transform();
        let p0 = t * line.p0;
        let p1 = t * line.p1;
        let (mut x0, mut y0, mut x1, mut y1) = (p0.x, p0.y, p1.x, p1.y);
        if let Some(clip) = self.active_clip() {
            let min_x = x0.min(x1);
            let max_x = x0.max(x1);
            let min_y = y0.min(y1);
            let max_y = y0.max(y1);
            // Reject lines fully outside the clip rect.
            if max_x < clip.x0 || min_x > clip.x1 || max_y < clip.y0 || min_y > clip.y1 {
                return;
            }
            // Trim axis-aligned segments to the clip so a clipped cell's
            // underline / strikethrough cannot spill into its neighbour. A
            // diagonal segment is left whole (none cross a clip today).
            if (y0 - y1).abs() < f64::EPSILON {
                x0 = x0.clamp(clip.x0, clip.x1);
                x1 = x1.clamp(clip.x0, clip.x1);
            } else if (x0 - x1).abs() < f64::EPSILON {
                y0 = y0.clamp(clip.y0, clip.y1);
                y1 = y1.clamp(clip.y0, clip.y1);
            }
        }
        self.renderer
            .push_line(Line::new((x0, y0), (x1, y1)), color, width as f32);
    }

    fn draw_glyphs(
        &mut self,
        run: &GlyphRun<'_, ParleyBrush>,
        _text: ambre_draw::LayoutText<'_>,
        brush: &Brush,
        transform: Affine,
    ) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return, // gradients: Phase 3
        };

        // Compose the caller's run transform with the active model-to-screen
        // transform so glyph origins land in screen space.
        let transform = self.transform() * transform;

        let parley_run = run.run();
        let font_data = parley_run.font();
        let font_size = Px::<crate::space::Parley>::new(parley_run.font_size());
        // swash reads the raw F2Dot14 bits, which parley 0.10 hands out as
        // they are. A static face has no coordinate, and an empty slice is free.
        let coords: &[i16] = parley_run.normalized_coords();
        let clip = self.active_clip();

        // A rotated run (Toile: a rotated sticky / text) rotates each glyph bitmap,
        // not just the baseline. The quad path is otherwise axis-aligned. For a
        // rotation we map the four glyph-quad corners through the transform's linear
        // part and emit a rotated quad. Detected from a non-zero off-diagonal.
        // Rotated board text carries no cell clip, so the axis-aligned clip-trim
        // path is skipped for it.
        let coeffs = transform.as_coeffs();
        let rotated = coeffs[1].abs() > 1e-9 || coeffs[2].abs() > 1e-9;
        let Some((scale, linear)) = split_linear(transform) else {
            return;
        };

        // The layout is logical, and the bitmap is not (`CANVAS-033`). A glyph
        // is rasterized at the transform's scale and the density, then drawn
        // back over the screen box that scale gives it (`CANVAS-074`).
        let density = self.renderer.density();
        let raster = raster_of_layout(font_size, scale, density);

        let mut pen_x = 0.0f32;
        for glyph in run.glyphs() {
            let gx = pen_x + glyph.x;
            let gy = glyph.y;
            pen_x += glyph.advance;

            let slot = match self.renderer.atlas.get_or_rasterize(
                font_data,
                raster,
                coords,
                GlyphId::new(glyph.id as u16),
            ) {
                Some(s) => s,
                None => continue,
            };

            if slot.pixel_width == 0 || slot.pixel_height == 0 {
                continue; // whitespace / invisible glyph
            }

            // Transform the glyph origin from run-local to screen space.
            let origin = transform * Point::new(gx as f64, gy as f64);
            let placement = GlyphPlacement {
                origin,
                linear: rotated.then_some(linear),
                density,
                clip,
                color,
            };
            emit_glyph(&mut *self.renderer, &slot, &placement);
        }
    }

    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {
        let Some(corners) = self.quad_corners(dest) else {
            return;
        };
        self.renderer
            .push_image(self.device, self.queue, image, corners);
    }

    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return, // gradients: Phase 3
        };
        let t = self.transform();
        if self.path_outside_clip(path, t) {
            return;
        }
        if let Some(mesh) = fill_mesh(path, t, self.tolerance(t)) {
            self.renderer
                .push_mesh(&mesh.vertices, &mesh.indices, color);
        }
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return, // gradients: Phase 3
        };
        let t = self.transform();
        // The stroke widens the fill bbox by half the line width. Grow the test
        // box so a thick stroke straddling the clip edge is not wrongly culled.
        if self.path_outside_clip_padded(path, t, stroke.width * 0.5) {
            return;
        }
        if let Some(mesh) = stroke_mesh(path, stroke, t, self.tolerance(t)) {
            self.renderer
                .push_mesh(&mesh.vertices, &mesh.indices, color);
        }
    }

    fn push_clip(&mut self, rect: Rect) {
        // Transform the clip into screen space at push time, so the stored
        // clip stack stays screen-space. The fast axis-aligned trimming in the
        // draw methods then needs no per-call transform. (Toile pan/zoom and
        // Prose margins are translate+scale, so an axis-aligned rect stays so;
        // a rotated transform would degrade to the transformed bounding box.)
        let rect = self.transform().transform_rect_bbox(rect);
        let clipped = match self.active_clip() {
            Some(p) => p.intersect(rect),
            None => rect,
        };
        self.clip_stack.push(clipped);
    }

    fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    fn push_transform(&mut self, transform: Affine) {
        // Compose onto the current transform so nested pushes accumulate
        // (for example a Toile pan/zoom outer transform with a per-shape inner one).
        self.transform_stack.push(self.transform() * transform);
    }

    fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }
}

// ============================================================================
// lyon tessellation helpers
// ============================================================================

/// A tessellated path: screen-space vertices and the triangles over them.
type Mesh = VertexBuffers<[f32; 2], u32>;

/// Tessellate the inside of `path` under the non-zero rule, at `tolerance`
/// in model units. Vertices come out mapped through `t` to the screen.
/// `None` when the tessellator refuses the path.
fn fill_mesh(path: &BezPath, t: Affine, tolerance: f32) -> Option<Mesh> {
    let mut mesh = Mesh::new();
    let opts = FillOptions::tolerance(tolerance).with_fill_rule(FillRule::NonZero);
    FillTessellator::new()
        .tessellate_path(
            &build_lyon_path(path),
            &opts,
            &mut BuffersBuilder::new(&mut mesh, |v: FillVertex| screen_vertex(v.position(), t)),
        )
        .ok()?;
    Some(mesh)
}

/// Tessellate `stroke` along `path`, at `tolerance` in model units.
/// Tessellating in model space keeps the line width in model units, so it
/// scales with `t` as a Toile zoom expects. `None` when the tessellator
/// refuses the path.
fn stroke_mesh(path: &BezPath, stroke: &Stroke, t: Affine, tolerance: f32) -> Option<Mesh> {
    let mut mesh = Mesh::new();
    let opts = StrokeOptions::tolerance(tolerance)
        .with_line_width(stroke.width as f32)
        .with_line_cap(line_cap(stroke.start_cap))
        .with_line_join(line_join(stroke.join))
        .with_miter_limit(stroke.miter_limit as f32);
    StrokeTessellator::new()
        .tessellate_path(
            &build_lyon_path(path),
            &opts,
            &mut BuffersBuilder::new(&mut mesh, |v: StrokeVertex| screen_vertex(v.position(), t)),
        )
        .ok()?;
    Some(mesh)
}

/// Build a lyon `Path` from a kurbo `BezPath`, mapping each element one-to-one.
/// Subpaths left open (no `ClosePath`) are ended unclosed so their edges are
/// still filled/stroked as an open contour.
fn build_lyon_path(path: &BezPath) -> LyonPath {
    let mut b = LyonPath::builder();
    let mut open = false;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if open {
                    b.end(false);
                }
                b.begin(lpoint(p.x as f32, p.y as f32));
                open = true;
            }
            // A `line_to`/curve before any `MoveTo` is malformed input; skip it
            // rather than panic inside the lyon builder.
            PathEl::LineTo(p) if open => {
                b.line_to(lpoint(p.x as f32, p.y as f32));
            }
            PathEl::QuadTo(c, p) if open => {
                b.quadratic_bezier_to(
                    lpoint(c.x as f32, c.y as f32),
                    lpoint(p.x as f32, p.y as f32),
                );
            }
            PathEl::CurveTo(c1, c2, p) if open => {
                b.cubic_bezier_to(
                    lpoint(c1.x as f32, c1.y as f32),
                    lpoint(c2.x as f32, c2.y as f32),
                    lpoint(p.x as f32, p.y as f32),
                );
            }
            PathEl::ClosePath if open => {
                b.end(true);
                open = false;
            }
            _ => {}
        }
    }
    if open {
        b.end(false);
    }
    b.build()
}

/// Map a lyon model-space vertex through the active transform to a screen-space
/// `[f32; 2]` position for the quad batch.
fn screen_vertex(p: lyon::math::Point, t: Affine) -> [f32; 2] {
    let sp = t * Point::new(p.x as f64, p.y as f64);
    [sp.x as f32, sp.y as f32]
}

/// Uniform scale factor of an affine transform (sqrt of the absolute
/// determinant of its linear part). Used to keep the flattening tolerance
/// constant in screen pixels regardless of Toile zoom.
fn affine_scale(t: Affine) -> f64 {
    let c = t.as_coeffs();
    (c[0] * c[3] - c[1] * c[2]).abs().sqrt()
}

fn line_cap(cap: kurbo::Cap) -> LineCap {
    match cap {
        kurbo::Cap::Butt => LineCap::Butt,
        kurbo::Cap::Round => LineCap::Round,
        kurbo::Cap::Square => LineCap::Square,
    }
}

fn line_join(join: kurbo::Join) -> LineJoin {
    match join {
        kurbo::Join::Miter => LineJoin::Miter,
        kurbo::Join::Round => LineJoin::Round,
        kurbo::Join::Bevel => LineJoin::Bevel,
    }
}

// ============================================================================
// Tests -- tessellation helpers (no GPU: exercise path build + lyon output).
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::GlyphRaster;

    fn ratio(r: f64) -> Scale<Css, Device> {
        Scale::<Css, Device>::new(r).expect("a positive density")
    }

    /// The area a mesh covers, summed over its triangles, after checking
    /// every index names a vertex.
    fn area(mesh: &Mesh) -> f64 {
        assert_eq!(mesh.indices.len() % 3, 0, "whole triangles");
        mesh.indices
            .chunks(3)
            .map(|tri| {
                let p = |i: u32| mesh.vertices[i as usize];
                let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
                f64::from(cross.abs()) / 2.0
            })
            .sum()
    }

    /// Records each glyph a run hands over, with the tint that routes it.
    #[derive(Default)]
    struct Batches {
        quads: Vec<(GlyphQuad, Option<Color>)>,
        corners: Vec<Option<Color>>,
    }

    impl GlyphSink for Batches {
        fn glyph(&mut self, quad: GlyphQuad, tint: Option<Color>) {
            self.quads.push((quad, tint));
        }
        fn glyph_corners(
            &mut self,
            _: [ambre_core::Point<Px<View>>; 4],
            _: &AtlasSlot,
            tint: Option<Color>,
        ) {
            self.corners.push(tint);
        }
    }

    /// A 10x10 slot standing on the baseline, a mask glyph or a colour one.
    fn glyph(atlas: AtlasKind) -> AtlasSlot {
        AtlasSlot {
            atlas,
            ..slot(0, 10, 10, 10)
        }
    }

    /// A run mixing a letter and an emoji paints both, each through its own
    /// atlas (`CANVAS-019`).
    ///
    /// The letter is tinted from the alpha atlas; the emoji is drawn untinted
    /// from the colour one, flat or rotated.
    #[test]
    fn a_mask_glyph_and_a_colour_glyph_land_in_their_own_batch() {
        let red = Color::from_rgb8(200, 0, 0);
        let at = |linear| GlyphPlacement {
            origin: Point::new(0.0, 10.0),
            linear,
            density: Scale::<Css, Device>::ONE,
            clip: None,
            color: red,
        };
        let mut batches = Batches::default();
        emit_glyph(&mut batches, &glyph(AtlasKind::Mask), &at(None));
        emit_glyph(&mut batches, &glyph(AtlasKind::Color), &at(None));
        let tints: Vec<_> = batches.quads.iter().map(|q| q.1).collect();
        assert_eq!(tints, vec![Some(red), None]);

        let turned = Some(Affine::rotate(0.5));
        emit_glyph(&mut batches, &glyph(AtlasKind::Mask), &at(turned));
        emit_glyph(&mut batches, &glyph(AtlasKind::Color), &at(turned));
        assert_eq!(batches.corners, vec![Some(red), None]);
    }

    /// A glyph straddling the clip is trimmed to it, and one outside is
    /// dropped.
    #[test]
    fn a_glyph_is_trimmed_to_the_clip() {
        let at = |x| GlyphPlacement {
            origin: Point::new(x, 10.0),
            linear: None,
            density: Scale::<Css, Device>::ONE,
            clip: Some(Rect::new(0.0, 0.0, 15.0, 20.0)),
            color: Color::BLACK,
        };
        let mut batches = Batches::default();
        emit_glyph(&mut batches, &glyph(AtlasKind::Mask), &at(10.0));
        emit_glyph(&mut batches, &glyph(AtlasKind::Mask), &at(20.0));
        assert_eq!(batches.quads.len(), 1, "the glyph past the clip is dropped");
        let quad = batches.quads[0].0;
        assert_eq!(
            [quad.x, quad.y, quad.width, quad.height].map(Px::<View>::get),
            [10.0, 0.0, 5.0, 10.0]
        );
    }

    /// A closed triangle with a base of 10 and a height of 10.
    fn triangle() -> BezPath {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 0.0));
        path.line_to((5.0, 10.0));
        path.close_path();
        path
    }

    /// A fill covers the path's area, in screen pixels. The transform is
    /// applied, so a scale of 3 covers nine times the area.
    #[test]
    fn a_fill_covers_the_path_in_screen_space() {
        let mesh = fill_mesh(&triangle(), Affine::IDENTITY, 0.1).expect("a mesh");
        assert!((area(&mesh) - 50.0).abs() < 1e-3, "{}", area(&mesh));
        let scaled = fill_mesh(&triangle(), Affine::scale(3.0), 0.1).expect("a mesh");
        assert!((area(&scaled) - 450.0).abs() < 1e-2, "{}", area(&scaled));
    }

    /// An open line strokes to a band of its length by its width. A butt
    /// cap adds nothing, and the transform scales the width with the length.
    #[test]
    fn a_stroke_covers_its_length_by_its_width() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((20.0, 0.0));
        let stroke = Stroke::new(2.0).with_caps(kurbo::Cap::Butt);
        let mesh = stroke_mesh(&path, &stroke, Affine::IDENTITY, 0.1).expect("a mesh");
        assert!((area(&mesh) - 40.0).abs() < 1e-3, "{}", area(&mesh));
        let scaled = stroke_mesh(&path, &stroke, Affine::scale(2.0), 0.1).expect("a mesh");
        assert!((area(&scaled) - 160.0).abs() < 1e-2, "{}", area(&scaled));
    }

    fn slot(left: i32, top: i32, w: u32, h: u32) -> AtlasSlot {
        AtlasSlot {
            uv_min: [0.0; 2],
            uv_max: [1.0; 2],
            pixel_width: w,
            pixel_height: h,
            bearing_left: left,
            bearing_top: top,
            atlas: AtlasKind::Mask,
            texel_scale: 1.0,
        }
    }

    /// A glyph is rasterized at the screen's resolution, and drawn back over
    /// its logical box (`CANVAS-033`). A raster at the logical size would be
    /// stretched over twice its texels on a dense screen.
    #[test]
    fn a_glyph_is_rasterized_at_the_physical_size() {
        assert_eq!(
            raster_of_layout(Px::<crate::space::Parley>::new(14.0), 1.0, ratio(2.0)),
            Px::<GlyphRaster>::new(28.0)
        );
        // A bitmap of 20 x 30 device pixels at density 2.
        assert_eq!(
            glyph_box(&slot(2, 24, 20, 30), ratio(2.0)),
            GlyphBox {
                left_bearing: Px::<View>::new(1.0),
                top_bearing: Px::<View>::new(12.0),
                width: Px::<View>::new(10.0),
                height: Px::<View>::new(15.0),
            }
        );
        assert_eq!(
            glyph_box(&slot(2, 24, 20, 30), Scale::<Css, Device>::ONE),
            GlyphBox {
                left_bearing: Px::<View>::new(2.0),
                top_bearing: Px::<View>::new(24.0),
                width: Px::<View>::new(20.0),
                height: Px::<View>::new(30.0),
            }
        );
    }

    /// A glyph made at the raster cap is drawn over the box of the size
    /// asked: each texel covers its scale in device pixels (`CANVAS-087`).
    #[test]
    fn a_glyph_made_at_the_cap_is_drawn_at_the_size_asked() {
        let capped = AtlasSlot {
            texel_scale: 3.0,
            ..slot(2, 24, 20, 30)
        };
        assert_eq!(
            glyph_box(&capped, ratio(2.0)),
            GlyphBox {
                left_bearing: Px::<View>::new(3.0),
                top_bearing: Px::<View>::new(36.0),
                width: Px::<View>::new(30.0),
                height: Px::<View>::new(45.0),
            }
        );
    }

    /// The raster a density change asks for is a different raster
    /// (`CANVAS-034`). The atlas keys by it, so no glyph made for the old
    /// screen is drawn on the new one.
    #[test]
    fn a_density_change_asks_for_a_new_raster() {
        use super::super::atlas::{FontId, GlyphKey};
        let key = |density| {
            let size = raster_of_layout(Px::<crate::space::Parley>::new(14.0), 1.0, ratio(density));
            GlyphKey::new(FontId::of_blob(7), GlyphId::new(42), size)
        };
        assert_ne!(key(1.0), key(2.0));
        assert_ne!(key(1.25), key(1.5));
        assert_eq!(key(2.0), key(2.0));
    }

    /// A run drawn under a zoom is rasterized at the size it covers
    /// (`CANVAS-074`). A rotated quad is then mapped by the rotation alone, or
    /// the zoom would apply twice.
    #[test]
    fn a_zoomed_glyph_is_rasterized_at_its_screen_size() {
        assert_eq!(
            raster_of_layout(
                Px::<crate::space::Parley>::new(14.0),
                2.0,
                Scale::<Css, Device>::ONE
            ),
            Px::<GlyphRaster>::new(28.0)
        );
        assert_eq!(
            raster_of_layout(Px::<crate::space::Parley>::new(14.0), 2.0, ratio(2.0)),
            Px::<GlyphRaster>::new(56.0)
        );

        let (scale, unit) =
            split_linear(Affine::translate((5.0, 7.0)) * Affine::scale(3.0) * Affine::rotate(0.5))
                .expect("a plane-preserving transform");
        assert!((scale - 3.0).abs() < 1e-12, "{scale}");
        let want = Affine::rotate(0.5).as_coeffs();
        for (got, want) in unit.as_coeffs().iter().zip(want) {
            assert!((got - want).abs() < 1e-12, "{unit:?}");
        }
        assert_eq!(split_linear(Affine::scale(0.0)), None);
    }

    /// The active transform scales tessellated positions into screen space.
    #[test]
    fn transform_maps_vertices_to_screen() {
        let t = Affine::translate((100.0, 50.0)) * Affine::scale(2.0);
        let v = screen_vertex(lpoint(3.0, 4.0), t);
        assert_eq!(v, [106.0, 58.0]);
    }
}
