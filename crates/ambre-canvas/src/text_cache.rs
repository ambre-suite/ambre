// ambre-canvas/src/text_cache.rs
// ============================================================================
// Cached Parley text layouts for UI rendering.
//
// Re-shaping text every frame is expensive. On a typical viewport with ~200
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

use crate::fonts::LazyFontLoader;

/// Map a font-family label to the face the application ships.
///
/// A thin wrapper over [`ambre_draw::resolve_family`], which is where the
/// mapping lives. The canvas renderers, the inline editors and the Prose
/// paginator all reach the same face for one label (`CANVAS-017`).
pub fn registered_family_name(label: Option<&str>) -> &'static str {
    ambre_draw::resolve_family(label).face_name()
}

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
    /// onto multiple lines instead of overflowing (`trame-cell-rendering`).
    break_word: bool,
    /// Font family name (`None` = the grid default). Part of the key so each
    /// family of the same string is cached separately.
    font: Option<Box<str>>,
}

// -----------------------------------------------------------------------------
// TextCache
// -----------------------------------------------------------------------------

/// A cached layout plus the monotonic tick of its last access, used to drive
/// least-recently-used eviction instead of clearing the whole cache.
struct CachedLayout {
    layout: Layout<Brush>,
    last_used: u64,
}

pub struct TextCache {
    layouts: FxHashMap<CacheKey, CachedLayout>,
    layout_cx: LayoutContext<Brush>,
    scale: f32,
    /// Monotonic access clock; bumped on every `get_or_build` so eviction can
    /// rank entries by recency. Wraps only after 2^64 accesses (never in
    /// practice).
    tick: u64,
    /// On-demand loader for scripts outside the base fonts (CJK, Arabic,
    /// Devanagari). Fed every string this cache shapes (on a miss); it fetches a
    /// face the first time it sees a codepoint the base fonts cannot cover.
    lazy_fonts: LazyFontLoader,
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
            tick: 0,
            lazy_fonts: LazyFontLoader::new(),
            frame_hits: 0,
            frame_misses: 0,
        }
    }

    /// Register any lazily-fetched font faces that arrived since the last frame
    /// into `font_cx`. If any did, clear the layout cache. Strings that shaped
    /// before the face was available (as tofu) then re-shape with real glyphs.
    ///
    /// Returns the faces registered, empty when coverage did not change. A
    /// non-empty return means the caller must force a full repaint: the cached
    /// GPU layers still hold the tofu pixels. It must also hand the same bytes
    /// to the threads that keep a context of their own (`CANVAS-068`).
    #[must_use]
    pub fn poll_lazy_fonts(&mut self, font_cx: &mut FontContext) -> Vec<Vec<u8>> {
        let faces = self.lazy_fonts.drain_into(font_cx);
        if !faces.is_empty() {
            self.layouts.clear();
        }
        faces
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

    /// Evict the least-recently-used entries when the cache exceeds
    /// `max_entries`, keeping the hottest layouts warm across frames.
    ///
    /// The previous implementation `clear()`ed the entire map on overflow. That
    /// threw away every still-visible cell's layout. The next frame paid a full
    /// re-shape, a visible stall right when the sheet is busiest. Instead we
    /// drop entries in ascending `last_used` order down to a low-water mark
    /// (75% of the cap). A single overflow frame then reshapes only the coldest
    /// quarter rather than everything.
    pub fn evict_if_oversized(&mut self, max_entries: usize) {
        if self.layouts.len() <= max_entries {
            return;
        }
        // Low-water mark: retain the hottest 75% so we do not re-run eviction
        // every frame once we hover near the cap.
        let target = max_entries - max_entries / 4;
        let to_remove = self.layouts.len() - target;
        // Partial selection of the `to_remove` smallest `last_used` ticks. A
        // full sort of the keys is O(n log n). Collecting the ticks and picking
        // a threshold is O(n), and enough here since caps are a few thousand.
        let mut ticks: Vec<u64> = self.layouts.values().map(|c| c.last_used).collect();
        // `select_nth_unstable` partitions so everything left of `to_remove`
        // is <= the pivot: that pivot is the eviction threshold.
        let threshold = *ticks.select_nth_unstable(to_remove - 1).1;
        // Retain entries strictly hotter than the threshold. Ties on the
        // threshold tick are kept, so we may retain slightly more than `target`
        // (harmless: the next overflow trims again).
        self.layouts.retain(|_, c| c.last_used > threshold);
    }

    /// Return a shaped `Layout` for `text`, building it on first access.
    ///
    /// `bold` / `italic` select the matching real face of the family, not faux
    /// bold/oblique. They are part of the cache key, so each style combination
    /// of the same string is stored separately.
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
    /// face, and `"Serif"`/`"Monospace"` to the matching generic. Any other name
    /// is requested verbatim and falls back to the default if unavailable.
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

        self.tick += 1;
        let tick = self.tick;

        use std::collections::hash_map::Entry;
        match self.layouts.entry(key) {
            Entry::Occupied(e) => {
                self.frame_hits += 1;
                let cached = e.into_mut();
                cached.last_used = tick;
                &cached.layout
            }
            Entry::Vacant(e) => {
                self.frame_misses += 1;
                // First shaping of this string: check it for codepoints outside
                // the base fonts and kick off a lazy fetch if needed. Only on a
                // miss -- a cache hit means we already scanned this text.
                self.lazy_fonts.observe(&e.key().text);
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
                &e.insert(CachedLayout {
                    layout,
                    last_used: tick,
                })
                .layout
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
        // Resolve every requested family to an actually-loaded face through the
        // single family authority. A family is a toolbar label
        // ("Sans"/"Serif"/...), the CSS-generic default ("sans-serif"), or an
        // unknown name. The authority is host.rs, which loads Noto Sans + DejaVu
        // Serif + DejaVu Sans Mono. Passing a non-label name straight to `Named`
        // (the old behaviour) matched no loaded face and shaped zero glyphs. A
        // sticky whose family was the generic default then rendered invisible text.
        // Edit mode -- which already routed through here -- showed it fine.
        let font_family = FontFamily::Named(registered_family_name(family).into());
        builder.push_default(StyleProperty::FontStack(FontStack::Single(font_family)));
        builder.push_default(StyleProperty::FontSize(font_size));
        if bold {
            builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        if italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        // Cell "wrap text": let an overlong unbreakable token break within the
        // word so it flows onto the next line (`trame-cell-rendering`).
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

#[cfg(test)]
mod tests {
    use super::registered_family_name;

    #[test]
    fn family_labels_map_to_registered_faces() {
        assert_eq!(registered_family_name(Some("Serif")), "DejaVu Serif");
        assert_eq!(
            registered_family_name(Some("Monospace")),
            "DejaVu Sans Mono"
        );
        // Sans / Default / None / any unknown label fall back to the bundled sans.
        assert_eq!(registered_family_name(Some("Sans")), "Noto Sans");
        assert_eq!(registered_family_name(Some("Default")), "Noto Sans");
        assert_eq!(registered_family_name(None), "Noto Sans");
        assert_eq!(registered_family_name(Some("Comic Sans")), "Noto Sans");
        // The CSS-generic `FontSpec` default MUST resolve to a loaded face, not be
        // passed through raw. Otherwise a sticky whose family is the default
        // shapes zero glyphs and renders invisible text. Edit mode already routed
        // through here and showed it fine. The static render bypassed it.
        assert_eq!(registered_family_name(Some("sans-serif")), "Noto Sans");
    }

    #[test]
    fn registered_face_names_pass_through() {
        // A request that already names a loaded face resolves to itself.
        // Routing every family through this single authority never demotes a
        // genuine serif/mono face to the sans fallback.
        assert_eq!(registered_family_name(Some("DejaVu Serif")), "DejaVu Serif");
        assert_eq!(
            registered_family_name(Some("DejaVu Sans Mono")),
            "DejaVu Sans Mono"
        );
    }
}
