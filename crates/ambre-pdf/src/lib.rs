// ambre-pdf/src/lib.rs
// ============================================================================
// KrillaSurface: the PDF-side `DrawSurface` implementation, backed by krilla.
//
// This is the export-time counterpart of `WgpuSurface` (ambre-canvas, screen).
// The same application render code (Trame grid, Prose page, Toile canvas) draws
// through `DrawSurface`; on screen it hits wgpu, on export it hits this surface
// and produces a PDF content stream. Screen and PDF must render identically,
// so nothing here reinterprets geometry -- it only translates each primitive to
// the matching krilla call.
//
// Placement: this executes in the Logic thread (not the UI thread) and the
// krilla dependency is lazy-loaded (WASM bundle splitting). See
// `ambre-spec-canvas.md` sec.D and `ambre-spec-architecture.md`.
//
// Coordinate system: krilla's surface origin is top-left with Y increasing
// downward -- the same convention as `DrawSurface`. krilla handles the PDF
// bottom-left Y-up flip internally, so no coordinate mapping is needed here.
// ============================================================================

use ambre_draw::{Affine, Brush, Color, DrawSurface, GlyphRun, ParleyBrush, Rect, Stroke};
use kurbo::{BezPath, PathEl};

use krilla::Data;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect as KRect, Transform};
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Paint, Stroke as KStroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};

/// Frame-scoped `DrawSurface` backed by a krilla page `Surface`.
///
/// Borrows the page surface for the duration of one page render. The caller
/// owns the krilla `Document`/`Page`; dropping the borrowed `Surface` (after
/// this wrapper is gone) flushes the content stream into the page.
///
/// The clip stack (`push_clip`/`pop_clip`) and the transform stack
/// (`push_transform`/`pop_transform`) both map onto krilla's single push/pop
/// instruction stack. `DrawSurface` guarantees these are balanced and properly
/// nested, so each `pop_*` corresponds to `Surface::pop` of the matching push.
pub struct KrillaSurface<'a, 'b> {
    surface: &'a mut Surface<'b>,
}

impl<'a, 'b> KrillaSurface<'a, 'b> {
    /// Wrap a krilla page surface. Obtain it via `page.surface()`.
    pub fn new(surface: &'a mut Surface<'b>) -> Self {
        Self { surface }
    }

    /// Fill the given krilla path with a solid color, honoring the color alpha
    /// as the fill opacity.
    fn fill_solid(&mut self, path: &krilla::geom::Path, color: Color) {
        let (paint, opacity) = paint_and_opacity(color);
        self.surface.set_fill(Some(Fill {
            paint,
            opacity,
            rule: FillRule::NonZero,
        }));
        self.surface.draw_path(path);
    }

    /// Stroke the given krilla path with a solid color and width. `cap`/`join`
    /// default to the krilla defaults (butt/miter); callers needing round joins
    /// go through `stroke_path`.
    fn stroke_solid(&mut self, path: &krilla::geom::Path, color: Color, width: f64) {
        let (paint, opacity) = paint_and_opacity(color);
        self.surface.set_stroke(Some(KStroke {
            paint,
            width: width as f32,
            opacity,
            ..KStroke::default()
        }));
        self.surface.draw_path(path);
    }
}

impl DrawSurface for KrillaSurface<'_, '_> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(path) = rect_path(rect) {
            self.fill_solid(&path, color);
        }
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64) {
        if let Some(path) = rect_path(rect) {
            self.stroke_solid(&path, color, width);
        }
    }

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0 as f32, y0 as f32);
        pb.line_to(x1 as f32, y1 as f32);
        if let Some(path) = pb.finish() {
            self.stroke_solid(&path, color, width);
        }
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return, // gradients: not yet supported on the PDF path
        };

        let parley_run = run.run();
        let font_data = parley_run.font();
        let font_size = parley_run.font_size();
        if font_size <= 0.0 {
            return;
        }

        // Rebuild the krilla font from the same bytes Parley shaped with. The
        // HTTP cache already deduplicates the download; here we only clone the
        // in-memory blob so krilla owns an independent handle. Variable-font
        // coordinates (`normalized_coords`) are ignored: the base export fonts
        // are static. Revisit for variable fonts (Font::new_variable).
        let font = match Font::new(Data::from(font_data.data.data().to_vec()), font_data.index) {
            Some(f) => f,
            None => return,
        };

        // krilla positions the run from `start`, then advances each glyph by its
        // (em-normalized) advance and applies per-glyph offsets. Parley already
        // gives per-glyph advance/offset in pixels at `font_size`, so normalize
        // by dividing by `font_size` (krilla multiplies back by the size).
        //
        // `text_range` is left empty: we do not thread the source string here,
        // so the PDF carries no ToUnicode mapping for these glyphs (text is not
        // selectable/copyable). Glyph shapes render correctly regardless. Wiring
        // the cluster text is a later refinement.
        let glyphs: Vec<KrillaGlyph> = run
            .glyphs()
            .map(|g| {
                KrillaGlyph::new(
                    GlyphId::new(g.id),
                    g.advance / font_size,
                    g.x / font_size,
                    g.y / font_size,
                    0.0,
                    0..0,
                    None,
                )
            })
            .collect();

        if glyphs.is_empty() {
            return;
        }

        let (paint, opacity) = paint_and_opacity(color);
        self.surface.set_fill(Some(Fill {
            paint,
            opacity,
            rule: FillRule::NonZero,
        }));

        // The run's origin transform (baseline placement, plus any Toile/Prose
        // pan/zoom) is applied as a pushed transform; glyphs then start at the
        // local origin. Balanced with the pop below.
        self.surface.push_transform(&affine_to_transform(transform));
        self.surface.draw_glyphs(
            Point::from_xy(0.0, 0.0),
            &glyphs,
            font,
            "",
            font_size,
            false,
        );
        self.surface.pop();
    }

    fn push_clip(&mut self, rect: Rect) {
        // Invariant: push_clip MUST emit exactly one push instruction so the
        // matching pop_clip (a bare Surface::pop) stays balanced with the
        // interleaved transform stack. `rect_path` yields None for a degenerate
        // (zero-area / inverted) rect; in that case we push a valid but
        // zero-area clip path -- which correctly clips everything out -- so the
        // count is preserved without a special case in pop_clip.
        match rect_path(rect) {
            Some(path) => self.surface.push_clip_path(&path, &FillRule::NonZero),
            None => self
                .surface
                .push_clip_path(&degenerate_clip_path(rect), &FillRule::NonZero),
        }
    }

    fn pop_clip(&mut self) {
        self.surface.pop();
    }

    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return,
        };
        if let Some(kpath) = bezpath_to_krilla(path) {
            self.fill_solid(&kpath, color);
        }
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return,
        };
        if let Some(kpath) = bezpath_to_krilla(path) {
            let (paint, opacity) = paint_and_opacity(color);
            self.surface.set_stroke(Some(KStroke {
                paint,
                width: stroke.width as f32,
                opacity,
                line_cap: line_cap(stroke.start_cap),
                line_join: line_join(stroke.join),
                ..KStroke::default()
            }));
            self.surface.draw_path(&kpath);
        }
    }

    fn push_transform(&mut self, transform: Affine) {
        self.surface.push_transform(&affine_to_transform(transform));
    }

    fn pop_transform(&mut self) {
        self.surface.pop();
    }
}

// ============================================================================
// Conversions -- ambre-draw (kurbo/peniko) -> krilla
// ============================================================================

/// Map a peniko sRGB color to a krilla RGB paint plus a normalized opacity.
/// `Color::components` are non-premultiplied sRGB channels in 0..=1.
fn paint_and_opacity(color: Color) -> (Paint, NormalizedF32) {
    let [r, g, b, a] = color.components;
    let paint = rgb::Color::new(to_u8(r), to_u8(g), to_u8(b)).into();
    let opacity = NormalizedF32::new(a.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE);
    (paint, opacity)
}

fn to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Build a krilla rectangle path. Returns `None` for a degenerate (empty or
/// inverted) rectangle, which krilla rejects.
fn rect_path(rect: Rect) -> Option<krilla::geom::Path> {
    let kr = KRect::from_ltrb(
        rect.x0 as f32,
        rect.y0 as f32,
        rect.x1 as f32,
        rect.y1 as f32,
    )?;
    let mut pb = PathBuilder::new();
    pb.push_rect(kr);
    pb.finish()
}

/// Build an always-valid, effectively zero-area clip path for a degenerate clip
/// rectangle. Used only by `push_clip` to preserve the push/pop balance when
/// `rect_path` rejects the rect. The extent is clamped to a sub-pixel epsilon so
/// `PathBuilder::finish` never returns `None` (a truly collapsed path would),
/// while the visible clip region stays negligible -- the intended "clip out
/// everything" behavior for a zero-area region.
fn degenerate_clip_path(rect: Rect) -> krilla::geom::Path {
    const EPS: f32 = 1e-4;
    let x = rect.x0.min(rect.x1) as f32;
    let y = rect.y0.min(rect.y1) as f32;
    let w = ((rect.x1 - rect.x0).abs() as f32).max(EPS);
    let h = ((rect.y1 - rect.y0).abs() as f32).max(EPS);
    let mut pb = PathBuilder::new();
    // from_xywh with a positive w/h always yields a valid rect; the unwrap is
    // therefore infallible (w, h >= EPS > 0).
    if let Some(kr) = KRect::from_xywh(x, y, w, h) {
        pb.push_rect(kr);
    }
    pb.finish()
        .expect("epsilon-sized rect path is always valid")
}

/// Convert a kurbo `BezPath` into a krilla path, element by element. Returns
/// `None` if the resulting path is empty (no fillable/strokable geometry).
fn bezpath_to_krilla(path: &BezPath) -> Option<krilla::geom::Path> {
    let mut pb = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, b) => pb.quad_to(a.x as f32, a.y as f32, b.x as f32, b.y as f32),
            PathEl::CurveTo(a, b, c) => pb.cubic_to(
                a.x as f32, a.y as f32, b.x as f32, b.y as f32, c.x as f32, c.y as f32,
            ),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

/// Map a kurbo affine transform to a krilla row-major transform.
/// kurbo `as_coeffs()` yields `[a, b, c, d, e, f]` for the matrix
/// `|a c e; b d f|`, which is exactly krilla's `from_row(sx, ky, kx, sy, tx, ty)`.
fn affine_to_transform(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(
        c[0] as f32,
        c[1] as f32,
        c[2] as f32,
        c[3] as f32,
        c[4] as f32,
        c[5] as f32,
    )
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
// Tests -- headless: draw primitives, serialize, assert a valid PDF.
// No GPU, no fonts required (glyph path exercised separately once shaping is
// wired into a Logic-thread test harness).
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use krilla::Document;
    use krilla::page::PageSettings;
    use kurbo::{Point as KurboPoint, Stroke as KurboStroke};
    use peniko::color::palette;

    /// Render a page exercising every geometry primitive plus the clip and
    /// transform stacks, then serialize. A well-formed PDF starts with "%PDF"
    /// and is non-trivial in size -- proof the content stream was written.
    #[test]
    fn renders_primitives_to_valid_pdf() {
        let mut doc = Document::new();
        let mut page = doc.start_page_with(PageSettings::from_wh(200.0, 200.0).unwrap());
        {
            let mut surface = page.surface();
            let mut s = KrillaSurface::new(&mut surface);

            s.fill_rect(Rect::new(10.0, 10.0, 90.0, 90.0), palette::css::RED);
            s.stroke_rect(Rect::new(100.0, 10.0, 190.0, 90.0), palette::css::BLUE, 2.0);
            s.stroke_line(10.0, 100.0, 190.0, 100.0, palette::css::BLACK, 1.0);

            // Clip + transform stacks, balanced.
            s.push_transform(Affine::translate((0.0, 100.0)));
            s.push_clip(Rect::new(0.0, 0.0, 100.0, 100.0));
            s.fill_rect(Rect::new(-50.0, -50.0, 150.0, 150.0), palette::css::GREEN);
            s.pop_clip();
            s.pop_transform();

            // Arbitrary path fill + stroke.
            let mut bez = BezPath::new();
            bez.move_to(KurboPoint::new(120.0, 120.0));
            bez.line_to(KurboPoint::new(180.0, 120.0));
            bez.quad_to(KurboPoint::new(180.0, 180.0), KurboPoint::new(120.0, 180.0));
            bez.close_path();
            s.fill_path(&bez, &Brush::Solid(palette::css::ORANGE));
            s.stroke_path(
                &bez,
                &Brush::Solid(palette::css::PURPLE),
                &KurboStroke::new(3.0),
            );
        }
        page.finish();

        let bytes = doc.finish().expect("PDF serialization must succeed");
        assert!(bytes.starts_with(b"%PDF"), "output must be a PDF");
        assert!(bytes.len() > 400, "content stream should be non-trivial");
    }

    /// A degenerate (inverted) rect produces no geometry but must not panic and
    /// must leave the surface usable.
    #[test]
    fn degenerate_rect_is_a_noop() {
        let mut doc = Document::new();
        let mut page = doc.start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
        {
            let mut surface = page.surface();
            let mut s = KrillaSurface::new(&mut surface);
            s.fill_rect(Rect::new(50.0, 50.0, 10.0, 10.0), palette::css::RED);
            s.fill_rect(Rect::new(20.0, 20.0, 80.0, 80.0), palette::css::BLUE);
        }
        page.finish();
        let bytes = doc.finish().expect("PDF serialization must succeed");
        assert!(bytes.starts_with(b"%PDF"));
    }

    /// Clip push/pop must stay balanced even for a zero-area clip rect.
    #[test]
    fn zero_area_clip_stays_balanced() {
        let mut doc = Document::new();
        let mut page = doc.start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
        {
            let mut surface = page.surface();
            let mut s = KrillaSurface::new(&mut surface);
            s.push_clip(Rect::new(50.0, 50.0, 50.0, 50.0));
            s.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), palette::css::RED);
            s.pop_clip();
            s.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), palette::css::GREEN);
        }
        page.finish();
        let bytes = doc.finish().expect("PDF serialization must succeed");
        assert!(bytes.starts_with(b"%PDF"));
    }
}
