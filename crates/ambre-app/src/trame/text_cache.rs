// ambre-app/src/trame/text_cache.rs
// ============================================================================
// Cached Parley text layouts for Trame grid rendering.
//
// Re-shaping text every frame is expensive -- on a typical viewport with ~200
// visible cells at 60 FPS that is 12 000 Parley shape calls per second.
//
// This cache stores Parley `Layout<Brush>` objects keyed by
// `(text, font_size_x100, max_width_x10, align)`.  After the first frame,
// header labels and repeated cell values are served from the cache with zero
// shaping cost.  Only new or changed strings trigger a shape call.
//
// The cache is stored in `GpuState` and lives for the lifetime of the app.
// Eviction is a full clear when the entry count exceeds a threshold -- simple
// and sufficient for the read-only grid we have today.
// ============================================================================

use parley::style::{FontFamily, FontStack, StyleProperty};
use parley::{Alignment, AlignmentOptions, FontContext, Layout, LayoutContext};
use rustc_hash::FxHashMap;
use vello::Scene;
use vello::kurbo::Affine;
use vello::peniko::{Brush, Color, Fill};

// -----------------------------------------------------------------------------
// Cache key
// -----------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    text: Box<str>,
    font_size_x100: u32,
    max_width_x10: u32,
    align: u8, // 0=Start, 1=Center, 2=End
}

// -----------------------------------------------------------------------------
// TextCache
// -----------------------------------------------------------------------------

pub struct TextCache {
    layouts: FxHashMap<CacheKey, Layout<Brush>>,
    layout_cx: LayoutContext<Brush>,
    scale: f32,
    // Per-frame diagnostics.
    pub frame_hits: usize,
    pub frame_misses: usize,
}

impl TextCache {
    pub fn new(scale: f32) -> Self {
        Self {
            layouts: FxHashMap::default(),
            layout_cx: LayoutContext::new(),
            scale,
            frame_hits: 0,
            frame_misses: 0,
        }
    }

    pub fn reset_frame_stats(&mut self) {
        self.frame_hits = 0;
        self.frame_misses = 0;
    }

    pub fn cache_size(&self) -> usize {
        self.layouts.len()
    }

    pub fn hit_rate(&self) -> f32 {
        let total = (self.frame_hits + self.frame_misses) as f32;
        if total == 0.0 { 0.0 } else { self.frame_hits as f32 / total }
    }

    /// Evict all entries when the cache exceeds `max_entries`.
    pub fn evict_if_oversized(&mut self, max_entries: usize) {
        if self.layouts.len() > max_entries {
            self.layouts.clear();
        }
    }

    // -------------------------------------------------------------------------
    // Render
    // -------------------------------------------------------------------------

    /// Shape (or serve from cache) and render `text` into `scene`, positioned
    /// within a cell rectangle `(cell_x, cell_y, cell_w, cell_h)`.
    ///
    /// Vertical centering is computed from the layout height.  Horizontal
    /// alignment is handled by Parley within `cell_w`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_cell_text(
        &mut self,
        scene: &mut Scene,
        font_cx: &mut FontContext,
        text: &str,
        cell_x: f64,
        cell_y: f64,
        cell_w: f64,
        cell_h: f64,
        font_size: f32,
        color: Color,
        align: Alignment,
    ) {
        if text.is_empty() {
            return;
        }

        let layout = self.get_or_build(font_cx, text, font_size, cell_w as f32, align);
        let text_h = layout.height() as f64;
        let y_offset = ((cell_h - text_h) / 2.0).max(0.0);
        let transform = Affine::translate((cell_x, cell_y + y_offset));
        let brush = Brush::Solid(color);
        render_layout(scene, layout, transform, &brush);
    }

    // -------------------------------------------------------------------------
    // Internal
    // -------------------------------------------------------------------------

    pub fn get_or_build(
        &mut self,
        font_cx: &mut FontContext,
        text: &str,
        font_size: f32,
        max_width: f32,
        align: Alignment,
    ) -> &Layout<Brush> {
        let align_u8 = match align {
            Alignment::Center => 1,
            Alignment::End | Alignment::Right => 2,
            _ => 0,
        };
        let key = CacheKey {
            text: text.into(),
            font_size_x100: (font_size * 100.0) as u32,
            max_width_x10: (max_width * 10.0) as u32,
            align: align_u8,
        };

        if self.layouts.contains_key(&key) {
            self.frame_hits += 1;
        } else {
            self.frame_misses += 1;
            let layout = Self::build_layout(
                &mut self.layout_cx, font_cx, text, font_size, max_width, align, self.scale,
            );
            self.layouts.insert(key.clone(), layout);
        }

        self.layouts.get(&key).unwrap()
    }

    fn build_layout(
        layout_cx: &mut LayoutContext<Brush>,
        font_cx: &mut FontContext,
        text: &str,
        font_size: f32,
        max_width: f32,
        align: Alignment,
        scale: f32,
    ) -> Layout<Brush> {
        let mut builder = layout_cx.ranged_builder(font_cx, text, scale, true);
        builder.push_default(StyleProperty::FontStack(FontStack::Single(
            FontFamily::Named("Noto Sans".into()),
        )));
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::Brush(Brush::Solid(Color::BLACK)));
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(max_width));
        layout.align(Some(max_width), align, AlignmentOptions::default());
        layout
    }
}

// -----------------------------------------------------------------------------
// Parley -> Vello bridge
// -----------------------------------------------------------------------------

/// Render a Parley `Layout<Brush>` into a Vello `Scene`.
///
/// `default_brush` overrides the stored brush (which is black by convention).
/// Each glyph run carries an offset (x) and baseline (y) that position it
/// within the line; individual glyph x/y are kerning offsets, and `advance`
/// must be accumulated to step the pen.
pub fn render_layout(
    scene: &mut Scene,
    layout: &Layout<Brush>,
    transform: Affine,
    default_brush: &Brush,
) {
    for line in layout.lines() {
        for item in line.items() {
            let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };

            let run = glyph_run.run();
            let font = run.font().clone();
            let font_size = run.font_size();
            let coords: Vec<_> = run.normalized_coords().to_vec();

            // Run position within the line.
            let run_transform =
                transform * Affine::translate((glyph_run.offset() as f64, glyph_run.baseline() as f64));

            scene
                .draw_glyphs(&font)
                .font_size(font_size)
                .transform(run_transform)
                .glyph_transform(None)
                .normalized_coords(&coords)
                .brush(default_brush)
                .hint(true)
                .draw(Fill::NonZero, {
                    let mut pen_x = 0.0_f32;
                    glyph_run.glyphs().map(move |g| {
                        let out = vello::Glyph { id: g.id, x: pen_x + g.x, y: g.y };
                        pen_x += g.advance;
                        out
                    })
                });
        }
    }
}
