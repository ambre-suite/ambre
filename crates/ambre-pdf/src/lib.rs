// ambre-pdf/src/lib.rs
// ============================================================================
// KrillaSurface: the PDF-side `DrawSurface` implementation, backed by krilla.
//
// This is the export-time counterpart of `WgpuSurface` (ambre-canvas, screen).
// The same application render code (Trame grid, Prose page, Toile canvas) draws
// through `DrawSurface`. On screen it hits wgpu; on export it hits this surface
// and produces a PDF content stream. Screen and PDF must render identically, so
// nothing here reinterprets geometry -- it only translates each primitive to
// the matching krilla call.
//
// Placement: this executes in the Logic thread (not the UI thread) and the
// krilla dependency is lazy-loaded (WASM bundle splitting). See
// `ambre-spec-drawing-surface.md` and `ambre-spec-architecture.md`.
//
// Coordinate system: krilla's surface origin is top-left with Y increasing
// downward -- the same convention as `DrawSurface`. krilla handles the PDF
// bottom-left Y-up flip internally, so no coordinate mapping is needed here.
// ============================================================================

mod document;
pub use document::{PdfWriteError, write_document};

use ambre_draw::Px;
use ambre_draw::list::{PrintGlyph, print_glyphs};
use ambre_draw::{
    Affine, Brush, Color, DrawSurface, EncodedFormat, EncodedImage, GlyphRun, ImageSource, Line,
    ParleyBrush, Rect, Stroke,
};
use kurbo::{BezPath, PathEl};

use krilla::Data;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect as KRect, Size as KSize, Transform};
use krilla::image::Image;
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

    /// Draw shaped glyphs of `font` at `size`, placed by `transform`. This is
    /// the one glyph path of the surface. A live parley run and a recorded
    /// draw list both end here, so the two print the same page.
    ///
    /// `text` is the text the glyphs draw, and each glyph's cluster cuts it.
    /// krilla writes a `ToUnicode` entry for a glyph that draws a cluster
    /// alone. It wraps the glyphs that share one in an `ActualText` span. A
    /// reader then selects and searches the words (`PDF-005`). A cluster that
    /// cuts `text` badly drops the characters of the whole run and keeps the
    /// shapes, since krilla would panic on it (`PDF-018`).
    pub fn draw_glyph_list(
        &mut self,
        font: Font,
        size: Px<ambre_draw::Surface>,
        glyphs: &[PrintGlyph],
        text: &str,
        color: Color,
        transform: Affine,
    ) {
        // krilla takes the size as `f32`.
        let font_size = size.get() as f32;
        if font_size <= 0.0 || glyphs.is_empty() {
            return;
        }
        let mapped = glyphs
            .iter()
            .all(|g| text.get(g.cluster.span().to_usize_range()).is_some());
        // krilla positions the run from `start`, then advances each glyph by its
        // (em-normalized) advance and applies per-glyph offsets. Parley already
        // gives per-glyph advance/offset in pixels at `font_size`, so normalize
        // by dividing by `font_size` (krilla multiplies back by the size).
        let glyphs: Vec<KrillaGlyph> = glyphs
            .iter()
            .map(|g| {
                KrillaGlyph::new(
                    GlyphId::new(g.id.get()),
                    g.advance.get() as f32 / font_size,
                    g.x.get() as f32 / font_size,
                    g.y.get() as f32 / font_size,
                    0.0,
                    if mapped {
                        g.cluster.span().to_usize_range()
                    } else {
                        0..0
                    },
                    None,
                )
            })
            .collect();
        let text = if mapped { text } else { "" };

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
            text,
            font_size,
            false,
        );
        self.surface.pop();
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
    fn stroke_solid(
        &mut self,
        path: &krilla::geom::Path,
        color: Color,
        width: Px<ambre_draw::Surface>,
    ) {
        let (paint, opacity) = paint_and_opacity(color);
        self.surface.set_stroke(Some(KStroke {
            paint,
            width: width.get() as f32,
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

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: Px<ambre_draw::Surface>) {
        if let Some(path) = rect_path(rect) {
            self.stroke_solid(&path, color, width);
        }
    }

    fn stroke_line(&mut self, line: Line, color: Color, width: Px<ambre_draw::Surface>) {
        let mut pb = PathBuilder::new();
        pb.move_to(line.p0.x as f32, line.p0.y as f32);
        pb.line_to(line.p1.x as f32, line.p1.y as f32);
        if let Some(path) = pb.finish() {
            self.stroke_solid(&path, color, width);
        }
    }

    fn draw_glyphs(
        &mut self,
        run: &GlyphRun<'_, ParleyBrush>,
        text: ambre_draw::LayoutText<'_>,
        brush: &Brush,
        transform: Affine,
    ) {
        let color = match brush {
            Brush::Solid(c) => *c,
            _ => return, // gradients: not yet supported on the PDF path
        };

        let parley_run = run.run();
        let font_data = parley_run.font();
        // Rebuild the krilla font from the same bytes Parley shaped with. The
        // HTTP cache already deduplicates the download; here we only clone the
        // in-memory blob so krilla owns an independent handle. Variable-font
        // coordinates (`normalized_coords`) are ignored: the base export fonts
        // are static. Revisit for variable fonts (Font::new_variable).
        let Some(font) = Font::new(Data::from(font_data.data.data().to_vec()), font_data.index)
        else {
            return;
        };
        let (glyphs, text) = print_glyphs(run, text);
        self.draw_glyph_list(
            font,
            Px::new(f64::from(parley_run.font_size())),
            &glyphs,
            &text,
            color,
            transform,
        );
    }

    fn push_clip(&mut self, rect: Rect) {
        // Invariant: push_clip MUST emit exactly one push instruction so the
        // matching pop_clip (a bare Surface::pop) stays balanced with the
        // interleaved transform stack. `rect_path` yields None for a degenerate
        // (zero-area / inverted) rect. In that case we push a valid but
        // zero-area clip path -- which correctly clips everything out. The
        // count is preserved without a special case in pop_clip. A rect with a
        // non-finite corner yields no path at all. An identity transform then
        // keeps the count instead: the page is left unclipped, never broken.
        match rect_path(rect).or_else(|| degenerate_clip_path(rect)) {
            Some(path) => self.surface.push_clip_path(&path, &FillRule::NonZero),
            None => self
                .surface
                .push_transform(&affine_to_transform(Affine::IDENTITY)),
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

    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {
        // krilla wants owned RGBA8 bytes; the source is a borrowed view, so copy
        // once into an image object krilla holds for the page's lifetime.
        let expected = image.size.width.get() as usize * image.size.height.get() as usize * 4;
        if image.pixels.as_bytes().len() != expected || expected == 0 {
            return; // malformed source: skip rather than embed garbage
        }
        let img = Image::from_rgba8(
            image.pixels.as_bytes().to_vec(),
            image.size.width.get(),
            image.size.height.get(),
        );
        // krilla draws the image occupying (0,0)..(size) in the current space, so
        // translate to the destination origin and size it to the rect. Balanced
        // push/pop, matching the transform-stack contract.
        let (w, h) = (dest.width() as f32, dest.height() as f32);
        let Some(size) = KSize::from_wh(w, h) else {
            return; // degenerate destination: nothing to paint
        };
        self.surface
            .push_transform(&affine_to_transform(Affine::translate((dest.x0, dest.y0))));
        self.surface.draw_image(img, size);
        self.surface.pop();
    }

    /// The file's bytes go into the PDF as they are: a JPEG is embedded
    /// whole, and a PNG is decoded here, in the PDF module. A file krilla
    /// refuses is skipped, as a malformed decoded source is.
    fn draw_encoded_image(&mut self, image: &EncodedImage<'_>, dest: Rect) {
        let data = Data::from(image.bytes.as_bytes().to_vec());
        let img = match image.format {
            EncodedFormat::Jpeg => Image::from_jpeg(data, true),
            EncodedFormat::Png => Image::from_png(data, true),
        };
        let Ok(img) = img else {
            return;
        };
        let (w, h) = (dest.width() as f32, dest.height() as f32);
        let Some(size) = KSize::from_wh(w, h) else {
            return;
        };
        self.surface
            .push_transform(&affine_to_transform(Affine::translate((dest.x0, dest.y0))));
        self.surface.draw_image(img, size);
        self.surface.pop();
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
    // Each channel reaches krilla as the 8-bit value its colour takes.
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let paint = rgb::Color::new(byte(r), byte(g), byte(b)).into();
    let opacity = NormalizedF32::new(a.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE);
    (paint, opacity)
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

/// Build an effectively zero-area clip path for a degenerate clip rectangle.
/// Used only by `push_clip` to preserve the push/pop balance when `rect_path`
/// rejects the rect. The extent is clamped to a sub-pixel epsilon, so only a
/// non-finite corner yields `None` (a truly collapsed path would too).
/// The visible clip region stays negligible -- the intended "clip out
/// everything" behavior for a zero-area region.
fn degenerate_clip_path(rect: Rect) -> Option<krilla::geom::Path> {
    const EPS: f32 = 1e-4;
    let x = rect.x0.min(rect.x1) as f32;
    let y = rect.y0.min(rect.y1) as f32;
    let w = ((rect.x1 - rect.x0).abs() as f32).max(EPS);
    let h = ((rect.y1 - rect.y0).abs() as f32).max(EPS);
    // A positive extent always yields a rect; only a non-finite corner fails.
    let kr = KRect::from_xywh(x, y, w, h)?;
    let mut pb = PathBuilder::new();
    pb.push_rect(kr);
    pb.finish()
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
    use ambre_draw::ImageKey;
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
            s.stroke_rect(
                Rect::new(100.0, 10.0, 190.0, 90.0),
                palette::css::BLUE,
                Px::<ambre_draw::Surface>::new(2.0),
            );
            s.stroke_line(
                ambre_draw::Line::new((10.0, 100.0), (190.0, 100.0)),
                palette::css::BLACK,
                Px::<ambre_draw::Surface>::new(1.0),
            );

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

    /// A raster image embeds into the page and enlarges the content stream. A
    /// tiny 2x2 RGBA source is enough to prove the `raster-images` path links and
    /// runs; a degenerate destination is skipped without panic.
    #[test]
    fn draws_a_raster_image() {
        let mut doc = Document::new();
        let mut page = doc.start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
        {
            let mut surface = page.surface();
            let mut s = KrillaSurface::new(&mut surface);
            // 2x2 opaque RGBA checkerboard.
            let pixels = ambre_draw::Payload::new(vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ]);
            let src = ImageSource {
                key: ImageKey::new(1),
                pixels: &pixels,
                size: ambre_core::Size::new(ambre_draw::Px::new(2), ambre_draw::Px::new(2)),
            };
            s.draw_image(&src, Rect::new(10.0, 10.0, 90.0, 90.0));
            // Degenerate destination: no-op, no panic.
            s.draw_image(&src, Rect::new(50.0, 50.0, 50.0, 50.0));
            // Malformed source (wrong length): skipped.
            let bad = ImageSource {
                key: ImageKey::new(2),
                pixels: &ambre_draw::Payload::new(vec![0, 1, 2]),
                size: ambre_core::Size::new(ambre_draw::Px::new(2), ambre_draw::Px::new(2)),
            };
            s.draw_image(&bad, Rect::new(0.0, 0.0, 10.0, 10.0));
        }
        page.finish();
        let bytes = doc.finish().expect("PDF serialization must succeed");
        assert!(bytes.starts_with(b"%PDF"));
        assert!(bytes.len() > 400, "an embedded image should grow the PDF");
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
