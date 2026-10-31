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

use ambre_bus::ByteSpan;
use ambre_core::{Doc, Px, View};

use ambre_draw::{Brush, BundledFamily, Color};
use parley::style::{
    FontFamily, FontFamilyName, FontStyle, FontWeight, OverflowWrap, StyleProperty,
};
use parley::{
    Alignment, AlignmentOptions, BreakReason, FontContext, Layout, LayoutContext, RangedBuilder,
    YieldData,
};
use rustc_hash::FxHashMap;

use crate::fonts::LazyFontLoader;
use crate::space::{Parley, WholeParley};

/// Map a font family to the face the application ships. `None` names the
/// default family.
///
/// A thin wrapper over [`ambre_draw::resolve_family`], which is where the
/// mapping lives. The canvas renderers, the inline editors and the Prose
/// paginator all reach the same face for one family (`CANVAS-017`).
pub fn registered_family_name(family: Option<&ambre_core::FontFamily>) -> &'static str {
    bundled_face(family).face_name()
}

/// The bundled face `family` resolves to, the default one for `None`.
fn bundled_face(family: Option<&ambre_core::FontFamily>) -> BundledFamily {
    match family {
        Some(family) => ambre_draw::resolve_family(family),
        None => ambre_draw::resolve_family(&ambre_core::FontFamily::Sans),
    }
}

// -----------------------------------------------------------------------------
// Cache key
// -----------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
    text: Box<str>,
    font_size_x100: u32,
    max_width_x10: u32,
    /// Each of the six alignments is its own entry: `Left` and `Start`
    /// differ in a right-to-left text, and `Justify` spreads its lines.
    align: Alignment,
    weight: Weight,
    slant: Slant,
    /// Whether a long unbreakable token wraps onto more lines instead of
    /// overflowing (`trame-cell-rendering`).
    word_break: WordBreak,
    /// The bundled face the family resolves to. Part of the key so each
    /// face of the same string is cached separately. Two families on one
    /// face shape the same layout, so they share the entry.
    face: BundledFamily,
    /// The ranges shaped in a face of their own, empty for a plain text.
    spans: Box<[SpanShape]>,
}

/// Parley's `Alignment` derives no `Hash`, so the key hashes its
/// discriminant; `Eq` still compares the alignment itself.
impl std::hash::Hash for CacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.font_size_x100.hash(state);
        self.max_width_x10.hash(state);
        std::mem::discriminant(&self.align).hash(state);
        self.weight.hash(state);
        self.slant.hash(state);
        self.word_break.hash(state);
        self.face.hash(state);
        self.spans.hash(state);
    }
}

pub use ambre_core::{Slant, Weight, WordBreak};

/// What a span shapes over its byte range: a weight, a slant, a family, an
/// underline, an indent. Its ink and its tint are drawn over the layout, so
/// they stay out of the cache key (`CANVAS-081`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SpanShape {
    pub range: ByteSpan,
    pub weight: Option<ambre_core::Bounded<ambre_core::FontWeight>>,
    pub italic: Slant,
    pub family: Option<ambre_core::FontFamily>,
    pub underline: ambre_core::Underline,
    /// How far right each paragraph holding bytes of the range starts, in
    /// the units of the width the text breaks to (`CANVAS-084`).
    pub indent: Px<WholeParley>,
}

/// Break `layout`, laid out from `text`, to `width`. Each paragraph holding
/// bytes of one of `indents` starts that far right, the largest one if
/// several hold it. Its lines narrow by as much, so a wrapped line keeps the
/// paragraph's edge (`CANVAS-084`). With no indent, the plain break. The
/// inline editor breaks its spans through it too.
pub(crate) fn break_indented<B: parley::Brush>(
    layout: &mut Layout<B>,
    text: &str,
    width: Px<Parley>,
    indents: &[(ByteSpan, Px<Parley>)],
) {
    let width = width.get();
    if !indents.iter().any(|(_, indent)| indent.get() > 0.0) {
        layout.break_all_lines(Some(width));
        return;
    }
    let shifts: Vec<f32> = paragraphs(text)
        .map(|held| {
            indents
                .iter()
                .filter(|(range, _)| !range.is_empty())
                .filter(|(range, _)| range.start <= held.end && held.start < range.end)
                .map(|(_, indent)| indent.get())
                .fold(0.0, f32::max)
                .min(width)
        })
        .collect();
    let mut lines = layout.break_lines();
    lines.state_mut().set_layout_max_advance(width);
    let mut paragraph = 0;
    loop {
        // The state set before a break is the one the line it commits takes.
        let shift = shifts.get(paragraph).copied().unwrap_or(0.0);
        let state = lines.state_mut();
        state.set_line_x(shift);
        state.set_line_max_advance(width - shift);
        match lines.break_next() {
            Some(YieldData::LineBreak(line)) => {
                if line.reason == BreakReason::Explicit {
                    paragraph += 1;
                }
            }
            // No max height is set, and no box of the canvas is out of flow.
            // Another yield ends the break rather than loop on it.
            Some(_) | None => break,
        }
    }
    lines.finish();
}

/// The byte ranges of the paragraphs of `text`, each break left out. A break
/// is a mandatory one of UAX #14, the set parley ends a paragraph on, and a
/// CR LF is one break.
fn paragraphs(text: &str) -> impl Iterator<Item = ByteSpan> + '_ {
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        if start > text.len() {
            return None;
        }
        while let Some((at, c)) = chars.next() {
            let mandatory = matches!(
                c,
                '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            );
            if mandatory {
                let mut next = at + c.len_utf8();
                if c == '\r' && chars.next_if(|(_, c)| *c == '\n').is_some() {
                    next += 1;
                }
                let held = ByteSpan::from_usize_range(start..at);
                start = next;
                return Some(held);
            }
        }
        let held = ByteSpan::from_usize_range(start..text.len());
        start = text.len() + 1;
        Some(held)
    })
}

/// Push the look of `shape` over its range, and say whether it was pushed.
/// A range that is empty or off a char boundary of `text` is not. The
/// inline editor shapes its spans through it too.
pub(crate) fn push_shape(
    builder: &mut RangedBuilder<'_, Brush>,
    text: &str,
    shape: &SpanShape,
) -> bool {
    let range = shape.range.to_usize_range();
    let placed =
        !range.is_empty() && text.is_char_boundary(range.start) && text.is_char_boundary(range.end);
    if !placed {
        return false;
    }
    if let Some(weight) = shape.weight {
        let weight = FontWeight::new(f32::from(weight.get()));
        builder.push(StyleProperty::FontWeight(weight), range.clone());
    }
    if shape.italic == Slant::Italic {
        builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
    }
    if let Some(family) = &shape.family {
        let face = registered_family_name(Some(family));
        let stack = FontFamily::Single(FontFamilyName::Named(face.into()));
        builder.push(StyleProperty::FontFamily(stack), range.clone());
    }
    if shape.underline == ambre_core::Underline::Underlined {
        builder.push(StyleProperty::Underline(true), range.clone());
    }
    true
}

// -----------------------------------------------------------------------------
// TextCache
// -----------------------------------------------------------------------------

/// An indent in the units of the width a layout breaks to: one whole unit
/// is one layout unit.
fn indent_units(indent: Px<WholeParley>) -> Px<Parley> {
    Px::new(f32::from(indent.get()))
}

/// A length a text is laid out in: a font size or a wrap width.
///
/// Parley lays out in the units of its caller, so a screen text passes
/// [`Px<View>`] and a board text passes [`Px<Doc>`]. Both lengths of one layout
/// share the space, so a font size in one space never meets a width in the
/// other. The length enters the layout as a [`Px<Parley>`].
pub trait TextLength: Copy {
    /// The length in the units parley lays out in.
    fn layout_units(self) -> Px<Parley>;
}

impl TextLength for Px<View> {
    fn layout_units(self) -> Px<Parley> {
        Px::new(self.get())
    }
}

impl TextLength for Px<Doc> {
    fn layout_units(self) -> Px<Parley> {
        Px::new(self.get())
    }
}

/// The look of a whole text: everything a layout is shaped with but the
/// text and its spans. Its two lengths share one [`TextLength`], unwrapped at
/// the parley call and the cache key only.
#[derive(Clone, Copy)]
struct Look<'a, L> {
    font_size: L,
    max_width: L,
    align: Alignment,
    weight: Weight,
    slant: Slant,
    family: Option<&'a ambre_core::FontFamily>,
    word_break: WordBreak,
}

/// A cached layout plus the monotonic tick of its last access, used to drive
/// least-recently-used eviction instead of clearing the whole cache.
struct CachedLayout {
    layout: Layout<Brush>,
    last_used: u64,
}

pub struct TextCache {
    layouts: FxHashMap<CacheKey, CachedLayout>,
    layout_cx: LayoutContext<Brush>,
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

impl Default for TextCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TextCache {
    /// An empty cache. It shapes in logical pixels, at a display scale of 1.
    /// A layout reasons in logical coordinates, and the frame applies the
    /// density when it draws (`CANVAS-032`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            layouts: FxHashMap::default(),
            layout_cx: LayoutContext::new(),
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
    pub fn poll_lazy_fonts(
        &mut self,
        font_cx: &mut FontContext,
    ) -> Vec<ambre_core::Payload<ambre_core::FontBytes>> {
        let faces = self.lazy_fonts.drain_into(font_cx);
        if !faces.is_empty() {
            self.layouts.clear();
        }
        faces
    }

    /// Number of lazy scripts a fetch has been started for, over every string
    /// shaped so far (test/diagnostic hook).
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn lazy_fonts_requested(&self) -> usize {
        self.lazy_fonts.requested_count()
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
    /// `weight` and `slant` select the matching real face of the family, not
    /// faux bold or oblique. They are part of the cache key, so each style combination
    /// of the same string is stored separately.
    #[allow(clippy::too_many_arguments)]
    pub fn get_or_build<L: TextLength>(
        &mut self,
        font_cx: &mut FontContext,
        text: &str,
        font_size: L,
        max_width: L,
        align: Alignment,
        weight: Weight,
        slant: Slant,
    ) -> &Layout<Brush> {
        self.get_or_build_font(
            font_cx,
            text,
            font_size,
            max_width,
            align,
            weight,
            slant,
            None,
            WordBreak::Whole,
        )
    }

    /// Like [`get_or_build`](Self::get_or_build) but selecting a font `family`
    /// (`None` = the grid default). The family resolves to a bundled face
    /// through [`registered_family_name`], and a family no face is named for
    /// falls back to the default one.
    #[allow(clippy::too_many_arguments)]
    pub fn get_or_build_font<L: TextLength>(
        &mut self,
        font_cx: &mut FontContext,
        text: &str,
        font_size: L,
        max_width: L,
        align: Alignment,
        weight: Weight,
        slant: Slant,
        family: Option<&ambre_core::FontFamily>,
        word_break: WordBreak,
    ) -> &Layout<Brush> {
        let plain = Look {
            font_size,
            max_width,
            align,
            weight,
            slant,
            family,
            word_break,
        };
        self.get_or_build_look(font_cx, text, plain, &[])
    }

    /// A body text at `font_size` within `max_width`, start-aligned, with
    /// each of `spans` shaped in its own look. Each set of spans of one text
    /// is cached apart (`CANVAS-081`).
    pub fn get_or_build_spans<L: TextLength>(
        &mut self,
        font_cx: &mut FontContext,
        text: &str,
        font_size: L,
        max_width: L,
        spans: &[SpanShape],
    ) -> &Layout<Brush> {
        let look = Look {
            font_size,
            max_width,
            align: Alignment::Start,
            weight: Weight::Regular,
            slant: Slant::Upright,
            family: None,
            word_break: WordBreak::Whole,
        };
        self.get_or_build_look(font_cx, text, look, spans)
    }

    fn get_or_build_look<L: TextLength>(
        &mut self,
        font_cx: &mut FontContext,
        text: &str,
        look: Look<'_, L>,
        spans: &[SpanShape],
    ) -> &Layout<Brush> {
        let Look {
            font_size,
            max_width,
            align,
            weight,
            slant,
            family,
            word_break,
        } = look;
        let key = CacheKey {
            text: text.into(),
            font_size_x100: (font_size.layout_units().get() * 100.0) as u32,
            max_width_x10: (max_width.layout_units().get() * 10.0) as u32,
            align,
            weight,
            slant,
            word_break,
            face: bundled_face(family),
            spans: spans.into(),
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
                let layout = Self::build_layout(
                    &mut self.layout_cx,
                    font_cx,
                    &e.key().text,
                    look,
                    &e.key().spans,
                );
                &e.insert(CachedLayout {
                    layout,
                    last_used: tick,
                })
                .layout
            }
        }
    }

    fn build_layout<L: TextLength>(
        layout_cx: &mut LayoutContext<Brush>,
        font_cx: &mut FontContext,
        text: &str,
        look: Look<'_, L>,
        spans: &[SpanShape],
    ) -> Layout<Brush> {
        let Look {
            font_size,
            max_width,
            align,
            weight,
            slant,
            family,
            word_break,
        } = look;
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        // Resolve every requested family to an actually-loaded face through the
        // single family authority. A family is a toolbar label
        // ("Sans"/"Serif"/...), the CSS-generic default ("sans-serif"), or an
        // unknown name. The authority is host.rs, which loads Noto Sans + DejaVu
        // Serif + DejaVu Sans Mono. Passing a non-label name straight to `Named`
        // (the old behaviour) matched no loaded face and shaped zero glyphs. A
        // sticky whose family was the generic default then rendered invisible text.
        // Edit mode -- which already routed through here -- showed it fine.
        let font_family = FontFamilyName::Named(registered_family_name(family).into());
        builder.push_default(StyleProperty::FontFamily(FontFamily::Single(font_family)));
        builder.push_default(StyleProperty::FontSize(font_size.layout_units().get()));
        if weight == Weight::Bold {
            builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        if slant == Slant::Italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        // Cell "wrap text": let an overlong unbreakable token break within the
        // word so it flows onto the next line (`trame-cell-rendering`).
        if word_break == WordBreak::Anywhere {
            builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::BreakWord));
        }
        builder.push_default(StyleProperty::Brush(Brush::Solid(Color::BLACK)));
        let mut indents = Vec::new();
        for shape in spans {
            if push_shape(&mut builder, text, shape) && shape.indent > Px::<WholeParley>::ZERO {
                indents.push((shape.range, indent_units(shape.indent)));
            }
        }
        let mut layout = builder.build(text);
        break_indented(&mut layout, text, max_width.layout_units(), &indents);
        layout.align(align, AlignmentOptions::default());
        layout
    }
}

#[cfg(test)]
mod tests {
    use super::{Slant, SpanShape, TextCache, Weight, registered_family_name};
    use ambre_bus::ByteSpan;
    use ambre_core::{Px, View};
    use parley::{Alignment, FontContext};

    /// The face the family `label` names resolves to.
    fn face_of(label: Option<&str>) -> &'static str {
        registered_family_name(label.map(ambre_core::FontFamily::from_label).as_ref())
    }

    /// A context holding the shipped Noto Sans face, read from the tree.
    fn font_cx() -> FontContext {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ambre-app/fonts/NotoSans-Regular.ttf"
        );
        let face = std::fs::read(path).unwrap_or_default();
        crate::fonts::font_context_from_bytes([ambre_core::Payload::new(face)])
    }

    /// Shape `text` through the cache and return its width and glyph count.
    fn shape(cache: &mut TextCache, cx: &mut FontContext, text: &str) -> (f32, usize) {
        let layout = cache.get_or_build(
            cx,
            text,
            Px::<View>::new(14.0),
            Px::<View>::new(400.0),
            Alignment::Start,
            Weight::Regular,
            Slant::Upright,
        );
        let glyphs = layout
            .lines()
            .flat_map(|line| line.items())
            .map(|item| match item {
                parley::PositionedLayoutItem::GlyphRun(run) => run.glyphs().count(),
                parley::PositionedLayoutItem::InlineBox(_) => 0,
            })
            .sum();
        (layout.width(), glyphs)
    }

    /// A span shapes its range in its own face. Each set of spans of one text
    /// is cached apart, and the same set is served again (`CANVAS-081`).
    #[test]
    fn each_set_of_spans_shapes_and_caches_apart() {
        const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        let faces = ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf"]
            .map(|file| std::fs::read(format!("{DIR}{file}")).expect("a shipped face"))
            .map(ambre_core::Payload::new);
        let mut cx = crate::fonts::font_context_from_bytes(faces);
        let mut cache = TextCache::new();
        let bold = SpanShape {
            range: ByteSpan::from_usize_range(0..4),
            weight: Some(ambre_core::Bounded::<ambre_core::FontWeight>::BOLD),
            ..SpanShape::default()
        };
        let faces_of = |layout: &parley::Layout<ambre_draw::Brush>| {
            let mut ids: Vec<u64> = layout
                .lines()
                .flat_map(|line| line.items())
                .filter_map(|item| match item {
                    parley::PositionedLayoutItem::GlyphRun(run) => Some(run.run().font().data.id()),
                    parley::PositionedLayoutItem::InlineBox(_) => None,
                })
                .collect();
            ids.dedup();
            ids.len()
        };
        let plain = cache.get_or_build_spans(
            &mut cx,
            "wide text",
            Px::<View>::new(14.0),
            Px::<View>::new(400.0),
            &[],
        );
        let plain_width = plain.width();
        assert_eq!(faces_of(plain), 1);
        let spanned = cache.get_or_build_spans(
            &mut cx,
            "wide text",
            Px::<View>::new(14.0),
            Px::<View>::new(400.0),
            std::slice::from_ref(&bold),
        );
        assert_eq!(faces_of(spanned), 2, "the span shaped no face of its own");
        assert!(spanned.width() > plain_width);
        let off = SpanShape {
            range: ByteSpan::from_usize_range(0..40),
            ..bold.clone()
        };
        let off = cache.get_or_build_spans(
            &mut cx,
            "wide text",
            Px::<View>::new(14.0),
            Px::<View>::new(400.0),
            &[off],
        );
        assert_eq!(faces_of(off), 1, "a range past the text shapes nothing");
        assert_eq!(cache.cache_size(), 3);
        cache.reset_frame_stats();
        cache.get_or_build_spans(
            &mut cx,
            "wide text",
            Px::<View>::new(14.0),
            Px::<View>::new(400.0),
            &[bold],
        );
        assert_eq!((cache.frame_hits, cache.frame_misses), (1, 0));
    }

    /// A span's indent starts each line of the paragraphs it holds that far
    /// right, a wrapped line too, and narrows them by as much. The paragraph
    /// after it starts at the edge again, and a CR LF is one break
    /// (`CANVAS-084`).
    #[test]
    fn an_indent_starts_each_line_of_its_paragraphs_past_it() {
        let mut cx = font_cx();
        let mut cache = TextCache::new();
        let long = "word ".repeat(20);
        let text = format!("top\r\n{long}\r\nend");
        let indent = SpanShape {
            range: ByteSpan::from_usize_range(5..5 + long.len() + 2),
            indent: Px::<crate::space::WholeParley>::new(12),
            ..SpanShape::default()
        };
        let layout = cache.get_or_build_spans(
            &mut cx,
            &text,
            Px::<View>::new(14.0),
            Px::<View>::new(200.0),
            &[indent],
        );
        let lines: Vec<(f32, f32)> = layout
            .lines()
            .map(|line| {
                let m = line.metrics();
                (
                    m.inline_min_coord,
                    m.inline_min_coord + m.advance - m.hanging_advance,
                )
            })
            .collect();
        let [(top, _), held @ .., (end, _)] = lines.as_slice() else {
            panic!("three paragraphs, got {lines:?}");
        };
        assert_eq!((*top, *end), (0.0, 0.0), "the paragraphs around it");
        assert!(held.len() >= 2, "the paragraph wraps, got {held:?}");
        assert!(
            held.iter()
                .all(|(start, end)| *start == 12.0 && *end <= 200.0),
            "each of its lines starts past the indent, within the width: {held:?}"
        );
        let paragraphs: Vec<_> = super::paragraphs("a\r\nb\n\u{2029}")
            .map(ByteSpan::to_usize_range)
            .collect();
        assert_eq!(paragraphs, [0..1, 3..4, 5..5, 8..8]);
    }

    /// An overflow drops the coldest layouts and keeps the hot ones
    /// (`CANVAS-022`).
    ///
    /// Clearing the cache whole would cost a full re-shape on the frame that
    /// is already the busiest.
    #[test]
    fn an_overflow_evicts_the_coldest_layouts_and_keeps_the_rest() {
        let mut cx = font_cx();
        let mut cache = TextCache::new();
        for i in 0..10 {
            shape(&mut cache, &mut cx, &format!("cell {i}"));
        }
        // Touch the two oldest, so they are the hottest now.
        shape(&mut cache, &mut cx, "cell 0");
        shape(&mut cache, &mut cx, "cell 1");

        cache.evict_if_oversized(8);
        assert_eq!(cache.cache_size(), 6, "the cache trims to three quarters");

        cache.reset_frame_stats();
        for hot in ["cell 0", "cell 1", "cell 9"] {
            shape(&mut cache, &mut cx, hot);
        }
        assert_eq!(cache.frame_misses, 0, "a hot layout survives the overflow");
        shape(&mut cache, &mut cx, "cell 2");
        assert_eq!(cache.frame_misses, 1, "the coldest layout is gone");
    }

    /// A layout that survives an eviction draws what it drew before
    /// (`CANVAS-054`).
    #[test]
    fn an_eviction_changes_nothing_a_surviving_layout_draws() {
        let mut cx = font_cx();
        let mut cache = TextCache::new();
        let before = shape(&mut cache, &mut cx, "Quarterly revenue");
        assert!(before.1 > 0, "the face shaped the string");
        for i in 0..10 {
            shape(&mut cache, &mut cx, &format!("filler {i}"));
        }
        shape(&mut cache, &mut cx, "Quarterly revenue");

        cache.evict_if_oversized(8);
        cache.reset_frame_stats();
        let after = shape(&mut cache, &mut cx, "Quarterly revenue");
        assert_eq!(cache.frame_hits, 1, "the layout survived");
        assert_eq!(after, before);
    }

    /// The frame that meets a missing script shapes at once and arms the
    /// fetch; the face lands on a later frame (`CANVAS-053`).
    ///
    /// Its arrival clears the layouts, which shaped without it.
    #[test]
    fn a_missing_script_arms_a_fetch_and_its_face_lands_later() {
        let mut cx = font_cx();
        let mut cache = TextCache::new();
        shape(&mut cache, &mut cx, "中文");
        assert_eq!(cache.lazy_fonts.requested_count(), 1, "the fetch is armed");
        assert_eq!(cache.cache_size(), 1, "the frame shaped without waiting");
        assert!(
            cache.poll_lazy_fonts(&mut cx).is_empty(),
            "nothing arrived yet"
        );

        cache.lazy_fonts.deliver(b"face".to_vec());
        assert_eq!(cache.poll_lazy_fonts(&mut cx).len(), 1);
        assert_eq!(cache.cache_size(), 0, "the stale layouts are dropped");
    }

    /// Each alignment of one text is cached apart. A justified text is never
    /// served the start-aligned layout (`CANVAS-086`).
    #[test]
    fn each_alignment_of_a_text_is_cached_apart() {
        let mut cx = font_cx();
        let mut cache = TextCache::new();
        let text = "word ".repeat(20);
        let all = [
            Alignment::Start,
            Alignment::End,
            Alignment::Left,
            Alignment::Center,
            Alignment::Right,
            Alignment::Justify,
        ];
        for align in all {
            cache.get_or_build(
                &mut cx,
                &text,
                Px::<View>::new(14.0),
                Px::<View>::new(120.0),
                align,
                Weight::Regular,
                Slant::Upright,
            );
        }
        assert_eq!(
            cache.cache_size(),
            all.len(),
            "two alignments shared an entry"
        );
        // Where the first line's last glyph lands: justify spreads the
        // words, so it lands further right than in the start-aligned line.
        let last_x = |layout: &parley::Layout<ambre_draw::Brush>| {
            layout
                .lines()
                .next()
                .into_iter()
                .flat_map(|line| line.items())
                .filter_map(|item| match item {
                    parley::PositionedLayoutItem::GlyphRun(run) => {
                        run.positioned_glyphs().last().map(|glyph| glyph.x)
                    }
                    parley::PositionedLayoutItem::InlineBox(_) => None,
                })
                .last()
        };
        let start = last_x(cache.get_or_build(
            &mut cx,
            &text,
            Px::<View>::new(14.0),
            Px::<View>::new(120.0),
            Alignment::Start,
            Weight::Regular,
            Slant::Upright,
        ));
        let justified = last_x(cache.get_or_build(
            &mut cx,
            &text,
            Px::<View>::new(14.0),
            Px::<View>::new(120.0),
            Alignment::Justify,
            Weight::Regular,
            Slant::Upright,
        ));
        assert!(justified > start, "the justified line was not spread");
    }

    #[test]
    fn family_labels_map_to_registered_faces() {
        assert_eq!(face_of(Some("Serif")), "DejaVu Serif");
        assert_eq!(face_of(Some("Monospace")), "DejaVu Sans Mono");
        // Sans / Default / None / any unknown label fall back to the bundled sans.
        assert_eq!(face_of(Some("Sans")), "Noto Sans");
        assert_eq!(face_of(Some("Default")), "Noto Sans");
        assert_eq!(face_of(None), "Noto Sans");
        assert_eq!(face_of(Some("Comic Sans")), "Noto Sans");
        // The CSS-generic `FontSpec` default MUST resolve to a loaded face, not be
        // passed through raw. Otherwise a sticky whose family is the default
        // shapes zero glyphs and renders invisible text. Edit mode already routed
        // through here and showed it fine. The static render bypassed it.
        assert_eq!(face_of(Some("sans-serif")), "Noto Sans");
    }

    #[test]
    fn registered_face_names_pass_through() {
        // A request that already names a loaded face resolves to itself.
        // Routing every family through this single authority never demotes a
        // genuine serif/mono face to the sans fallback.
        assert_eq!(face_of(Some("DejaVu Serif")), "DejaVu Serif");
        assert_eq!(face_of(Some("DejaVu Sans Mono")), "DejaVu Sans Mono");
    }
}
