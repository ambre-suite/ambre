// ambre-canvas/src/render/surface.rs
// ============================================================================
// WgpuSurface: the frame-scoped `DrawSurface` adapter over `QuadRenderer`. Owns
// the clip stack and routes fill/stroke/glyph pushes to the renderer active
// layer, with per-glyph atlas rasterization and clip trimming.
// ============================================================================

use super::*;
use ambre_draw::{Affine, Brush, Color, DrawSurface, GlyphRun, ParleyBrush, Point, Rect};

// ============================================================================
// WgpuSurface
// ============================================================================

/// Frame-scoped `DrawSurface` backed by `QuadRenderer`. Push routing follows
/// `renderer.active_layer`, set by the caller before each phase.
pub struct WgpuSurface<'r> {
    renderer: &'r mut QuadRenderer,
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
        // Picture-frame decomposition: two full-width horizontal bars (which
        // own the four corners) plus two verticals INSET to sit strictly
        // between them. The quads share only zero-area edges, so no pixel is
        // painted twice -- a translucent stroke keeps a uniform alpha instead of
        // the doubled corners a naive 4-overlapping-quad frame produces.
        self.fill_rect(
            Rect::new(rect.x0 - hw, rect.y0 - hw, rect.x1 + hw, rect.y0 + hw),
            color,
        );
        self.fill_rect(
            Rect::new(rect.x0 - hw, rect.y1 - hw, rect.x1 + hw, rect.y1 + hw),
            color,
        );
        // Skip the inset verticals when the rect is shorter than the stroke:
        // the two horizontal bars already overlap and cover the whole span, and
        // an inverted `y0 + hw > y1 - hw` rect would otherwise produce garbage.
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

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        let (mut x0, mut y0, mut x1, mut y1) = (x0, y0, x1, y1);
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

            // Clip the glyph quad to the active clip rect. A glyph straddling an
            // edge must be *trimmed* (with its UVs remapped proportionally), not
            // drawn whole -- otherwise an overflowing cell's last glyph spills a
            // full character into its neighbour (sec.4.A).
            let (x0, y0, x1, y1) = (sx, sy, sx + sw, sy + sh);
            if let Some(clip) = clip {
                let (cx0, cy0) = (clip.x0 as f32, clip.y0 as f32);
                let (cx1, cy1) = (clip.x1 as f32, clip.y1 as f32);
                // Fully outside: skip.
                if x1 <= cx0 || x0 >= cx1 || y1 <= cy0 || y0 >= cy1 {
                    continue;
                }
                // Clamp the quad, then map the clamped extents back to UV space
                // by the (axis-aligned) linear glyph mapping.
                let (nx0, nx1) = (x0.max(cx0), x1.min(cx1));
                let (ny0, ny1) = (y0.max(cy0), y1.min(cy1));
                let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
                let u0 = lerp(slot.uv_min[0], slot.uv_max[0], (nx0 - x0) / sw);
                let u1 = lerp(slot.uv_min[0], slot.uv_max[0], (nx1 - x0) / sw);
                let v0 = lerp(slot.uv_min[1], slot.uv_max[1], (ny0 - y0) / sh);
                let v1 = lerp(slot.uv_min[1], slot.uv_max[1], (ny1 - y0) / sh);
                self.renderer
                    .push_glyph(nx0, ny0, nx1 - nx0, ny1 - ny0, [u0, v0], [u1, v1], color);
            } else {
                self.renderer
                    .push_glyph(sx, sy, sw, sh, slot.uv_min, slot.uv_max, color);
            }
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
