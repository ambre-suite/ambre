// ambre-canvas/src/text_cache.rs
// ============================================================================
// Cached Parley text layouts for UI rendering.
//
// Re-shaping text every frame is expensive: on a typical viewport with ~200
// visible cells at 60 FPS that would be 12 000 Parley shape calls per second.
//
// This cache stores Parley `Layout<Brush>` objects keyed by
// `(text, font_size_x100, max_width_x10, align)`. After the first frame,
// header labels and repeated cell values are served from the cache with zero
// shaping cost. Only new or changed strings trigger a shape call.
// ============================================================================

use ambre_draw::{Brush, Color};
use parley::style::{FontFamily, FontStack, FontStyle, FontWeight, OverflowWrap, StyleProperty};
use parley::{Alignment, AlignmentOptions, FontContext, Layout, LayoutContext};
use rustc_hash::FxHashMap;

// -----------------------------------------------------------------------------
// Cache key
// -----------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    text: Box<str>,
    font_size_x100: u32,
    max_width_x10: u32,
    align: u8, // 0=Start, 1=Center, 2=End
    bold: bool,
    italic: bool,
    /// Allow emergency (within-word) breaking so a long unbreakable token wraps
    /// onto multiple lines instead of overflowing (cell "wrap text", sec.4.A).
    break_word: bool,
    /// Font family name (`None` = the grid default). Part of the key so each
    /// family of the same string is cached separately.
    font: Option<Box<str>>,
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
        if total == 0.0 {
            0.0
        } else {
            self.frame_hits as f32 / total
        }
    }

    /// Evict all entries when the cache exceeds `max_entries`.
    pub fn evict_if_oversized(&mut self, max_entries: usize) {
        if self.layouts.len() > max_entries {
            self.layouts.clear();
        }
    }

    /// Return a shaped `Layout` for `text`, building it on first access.
    ///
    /// `bold` / `italic` select the matching real face of the family (not faux
    /// bold/oblique) and are part of the cache key, so each style combination of
    /// the same string is stored separately.
    #[allow(clippy::too_many_arguments)]
    pub fn get_or_build(
        &mut self,
        font_cx: &mut FontContext,
        text: &str,
        font_size: f32,
        max_width: f32,
        align: Alignment,
        bold: bool,
        italic: bool,
    ) -> &Layout<Brush> {
        self.get_or_build_font(
            font_cx, text, font_size, max_width, align, bold, italic, None, false,
        )
    }

    /// Like [`get_or_build`](Self::get_or_build) but selecting a font `family`
    /// (`None` = the grid default). `"Sans"`/`"Default"` map to the bundled sans
    /// face; `"Serif"`/`"Monospace"` to the matching generic; any other name is
    /// requested verbatim and falls back to the default if unavailable.
    #[allow(clippy::too_many_arguments)]
    pub fn get_or_build_font(
        &mut self,
        font_cx: &mut FontContext,
        text: &str,
        font_size: f32,
        max_width: f32,
        align: Alignment,
        bold: bool,
        italic: bool,
        family: Option<&str>,
        break_word: bool,
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
            bold,
            italic,
            break_word,
            font: family.map(Box::from),
        };

        use std::collections::hash_map::Entry;
        match self.layouts.entry(key) {
            Entry::Occupied(e) => {
                self.frame_hits += 1;
                e.into_mut()
            }
            Entry::Vacant(e) => {
                self.frame_misses += 1;
                let family = e.key().font.clone();
                let layout = Self::build_layout(
                    &mut self.layout_cx,
                    font_cx,
                    &e.key().text,
                    font_size,
                    max_width,
                    align,
                    bold,
                    italic,
                    family.as_deref(),
                    break_word,
                    self.scale,
                );
                e.insert(layout)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_layout(
        layout_cx: &mut LayoutContext<Brush>,
        font_cx: &mut FontContext,
        text: &str,
        font_size: f32,
        max_width: f32,
        align: Alignment,
        bold: bool,
        italic: bool,
        family: Option<&str>,
        break_word: bool,
        scale: f32,
    ) -> Layout<Brush> {
        let mut builder = layout_cx.ranged_builder(font_cx, text, scale, true);
        // Map the toolbar's family choices to the actually-registered faces
        // (host.rs loads Noto Sans + DejaVu Serif + DejaVu Sans Mono). The
        // generic fallback covers any other named family.
        let font_family = match family {
            Some("Serif") => FontFamily::Named("DejaVu Serif".into()),
            Some("Monospace") => FontFamily::Named("DejaVu Sans Mono".into()),
            Some(name) if name != "Sans" && name != "Default" => FontFamily::Named(name.into()),
            _ => FontFamily::Named("Noto Sans".into()),
        };
        builder.push_default(StyleProperty::FontStack(FontStack::Single(font_family)));
        builder.push_default(StyleProperty::FontSize(font_size));
        if bold {
            builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        if italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        // Cell "wrap text": let an overlong unbreakable token break within the
        // word so it flows onto the next line instead of overflowing (sec.4.A).
        if break_word {
            builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::BreakWord));
        }
        builder.push_default(StyleProperty::Brush(Brush::Solid(Color::BLACK)));
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(max_width));
        layout.align(Some(max_width), align, AlignmentOptions::default());
        layout
    }
}
