// ambre-draw/src/shaping.rs
// ============================================================================
// The one constructor of a ranged text layout (`CANVAS-090`).
//
// parley 0.10 maps a cluster on the first family of its style that holds the
// base character. It counts a presentation selector `U+FE0F` as mapped.
// So a base face claims `U+2764 U+FE0F` whole: it draws glyph 0 for the heart,
// or its own monochrome glyph for `U+263A`. The fallback table never sees the
// cluster, since the base face already answered it.
//
// The fix is in the style, not in the shaper. Each emoji sequence that holds a
// presentation selector takes a family list of its own. The generic emoji
// family comes first, then the base family it sits in. The base family still draws
// the sequence until the emoji face arrives, since the generic family is empty
// before then.
//
// Every layout of the workspace starts here. So the screen, the page measure
// and the PDF export shape one sequence on one face. The
// driver of `ambre-primitives` refuses a parley builder called anywhere else.
// ============================================================================

use ambre_core::Byte;
use ambre_core::index::Span;
use parley::{
    Brush, FontContext, FontFamily, FontFamilyName, GenericFamily, InlineBox, Layout,
    LayoutContext, RangedBuilder, StyleProperty,
};

use crate::text::BundledFamily;

/// A layout under construction: the parley builder, and the family of each
/// stretch of the text.
///
/// The family is set by [`text_builder`] and [`TextBuilder::push_family`]
/// alone. [`TextBuilder::build`] then knows the base family of each emoji
/// sequence, and pushes its family list last, over every other span.
#[must_use]
pub struct TextBuilder<'a, B: Brush> {
    builder: RangedBuilder<'a, B>,
    text: &'a str,
    family: BundledFamily,
    families: Vec<(Span<Byte>, BundledFamily)>,
}

/// Start the layout of `text` in `family`, at scale one and with quantized
/// positions, as every layout of the workspace is built (`CANVAS-090`).
///
/// This is the only call of a parley builder in the workspace: the primitive
/// gate refuses any other.
pub fn text_builder<'a, B: Brush>(
    layout_cx: &'a mut LayoutContext<B>,
    font_cx: &'a mut FontContext,
    text: &'a str,
    family: BundledFamily,
) -> TextBuilder<'a, B> {
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(single(family)));
    TextBuilder {
        builder,
        text,
        family,
        families: Vec::new(),
    }
}

impl<B: Brush> TextBuilder<'_, B> {
    /// Set `property` on the whole text.
    ///
    /// A family property is dropped: the family comes from [`text_builder`],
    /// so that an emoji sequence knows its base family.
    pub fn push_default<'p>(&mut self, property: impl Into<StyleProperty<'p, B>>) {
        let property = property.into();
        if !matches!(property, StyleProperty::FontFamily(_)) {
            self.builder.push_default(property);
        }
    }

    /// Set `property` on `span` of the text.
    ///
    /// A family property is dropped: [`TextBuilder::push_family`] sets the
    /// family of a span.
    pub fn push<'p>(&mut self, property: impl Into<StyleProperty<'p, B>>, span: Span<Byte>) {
        let property = property.into();
        if !matches!(property, StyleProperty::FontFamily(_)) {
            self.builder.push(property, span.to_usize_range());
        }
    }

    /// Set the family of `span` of the text.
    pub fn push_family(&mut self, family: BundledFamily, span: Span<Byte>) {
        self.builder.push(
            StyleProperty::FontFamily(single(family)),
            span.to_usize_range(),
        );
        self.families.push((span, family));
    }

    /// Add an inline box to the layout.
    pub fn push_inline_box(&mut self, inline_box: InlineBox) {
        self.builder.push_inline_box(inline_box);
    }

    /// Shape the text. Each emoji sequence closed by a presentation selector
    /// takes the emoji family before its base family (`CANVAS-090`).
    pub fn build(mut self) -> Layout<B> {
        for sequence in emoji_sequences(self.text) {
            // The last family pushed over the sequence wins, as it does in
            // parley.
            let base = self
                .families
                .iter()
                .rev()
                .find(|(span, _)| span.contains(sequence.start))
                .map_or(self.family, |&(_, family)| family);
            let stack = [
                FontFamilyName::Generic(GenericFamily::Emoji),
                FontFamilyName::Named(base.face_name().into()),
            ];
            self.builder.push(
                StyleProperty::FontFamily(FontFamily::List(stack[..].into())),
                sequence.to_usize_range(),
            );
        }
        self.builder.build(self.text)
    }
}

/// The family property that names `family` alone.
fn single(family: BundledFamily) -> FontFamily<'static> {
    FontFamily::Single(FontFamilyName::Named(family.face_name().into()))
}

/// Whether `text` holds an emoji sequence closed by a presentation selector.
///
/// The inline editor lays such a text out through [`text_builder`], since
/// the parley editor builds its own layout with one family.
#[must_use]
pub fn holds_emoji_sequence(text: &str) -> bool {
    emoji_sequences(text).next().is_some()
}

/// The emoji sequences of `text` that hold a presentation selector, in order.
///
/// A sequence is a character, then each selector, keycap, skin tone and tag
/// after it, then each joiner with the character it joins. A sequence with no
/// presentation selector is left alone: the fallback table already finds its
/// face (`CANVAS-089`).
fn emoji_sequences(text: &str) -> impl Iterator<Item = Span<Byte>> + '_ {
    // Nearly every text holds no selector, and this scan is all it costs.
    let mut chars = text
        // The emoji presentation selector.
        .contains('\u{FE0F}')
        .then(|| text.char_indices())
        .into_iter()
        .flatten()
        .peekable();
    core::iter::from_fn(move || {
        while let Some((start, first)) = chars.next() {
            let mut end = start + first.len_utf8();
            let mut selected = false;
            while let Some(&(at, next)) = chars.peek() {
                match next {
                    // The presentation selector asks for the colour glyph.
                    '\u{FE0F}' => selected = true,
                    // The zero width joiner glues the next emoji on.
                    '\u{200D}' => {
                        chars.next();
                        end = at + next.len_utf8();
                        if let Some((joined_at, joined)) = chars.next() {
                            end = joined_at + joined.len_utf8();
                        }
                        continue;
                    }
                    '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}' => {}
                    _ => break,
                }
                chars.next();
                end = at + next.len_utf8();
            }
            if selected {
                return Some(Span::from_usize_range(start..end));
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use ambre_core::Byte;
    use ambre_core::index::Span;
    use parley::{FontContext, FontFamily, LayoutContext, PositionedLayoutItem, StyleProperty};

    use super::{emoji_sequences, holds_emoji_sequence, text_builder};
    use crate::ParleyBrush;
    use crate::text::fallback_tests::{context_with_all, read};
    use crate::text::{BundledFamily, LazyFont};

    const FAMILIES: [BundledFamily; 3] = [
        BundledFamily::Sans,
        BundledFamily::Serif,
        BundledFamily::Mono,
    ];

    /// One glyph as shaped: its id, and the size of the face file it came
    /// from, which tells the shipped faces apart.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    struct Shaped {
        glyph: u32,
        face: usize,
    }

    /// The glyphs of `text` built by `build`, in visual order.
    fn shaped(
        font_cx: &mut FontContext,
        text: &str,
        build: impl FnOnce(&mut super::TextBuilder<'_, ParleyBrush>),
    ) -> Vec<Shaped> {
        let mut layout_cx: LayoutContext<ParleyBrush> = LayoutContext::new();
        let mut builder = text_builder(&mut layout_cx, font_cx, text, BundledFamily::Sans);
        builder.push_default(StyleProperty::FontSize(16.0));
        build(&mut builder);
        let mut layout = builder.build();
        layout.break_all_lines(None);
        let mut out = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    let face = run.run().font().data.as_ref().len();
                    out.extend(run.glyphs().map(|g| Shaped { glyph: g.id, face }));
                }
            }
        }
        out
    }

    /// The size of a shipped face file.
    fn face_len(file: &str) -> usize {
        read(file).into_bytes().len()
    }

    fn emoji_len() -> usize {
        face_len("NotoColorEmoji.ttf")
    }

    /// `U+2764 U+FE0F` and `U+263A U+FE0F` draw one colour glyph from the
    /// emoji face under each base family, alone and inside a word
    /// (`CANVAS-090`). Before the fix the first drew glyph 0, and the second
    /// the monochrome glyph of the serif and mono faces.
    #[test]
    fn an_emoji_closed_by_a_selector_draws_from_the_emoji_face_under_every_base_family() {
        let mut font_cx = context_with_all(&[LazyFont::Emoji]);
        let emoji = emoji_len();
        for family in FAMILIES {
            for (text, emoji_glyphs) in [
                ("\u{2764}\u{FE0F}", 1),
                ("\u{263A}\u{FE0F}", 1),
                ("ok \u{2764}\u{FE0F} ok", 1),
                ("\u{2764}\u{FE0F}\u{263A}\u{FE0F}", 2),
                ("\u{2764}\u{FE0F}\u{200D}\u{1F525}", 1),
            ] {
                let glyphs = shaped(&mut font_cx, text, |b| {
                    b.push_family(family, Span::from_usize_range(0..text.len()));
                });
                let from_emoji: Vec<_> = glyphs.iter().filter(|g| g.face == emoji).collect();
                assert_eq!(
                    from_emoji.len(),
                    emoji_glyphs,
                    "`{}` under {family:?}: {glyphs:?}",
                    text.escape_unicode()
                );
                assert!(
                    glyphs.iter().all(|g| g.glyph != 0),
                    "`{}` under {family:?} drew tofu: {glyphs:?}",
                    text.escape_unicode()
                );
            }
        }
    }

    /// A bare `U+2764` shapes as before the fix: the parley 0.12 spike
    /// regressed it. The sans and the serif lack it, so the fallback table
    /// finds the emoji face. The mono holds a text glyph of its own, which a
    /// bare heart asks for.
    #[test]
    fn a_bare_heart_shapes_as_before() {
        let mut font_cx = context_with_all(&[LazyFont::Emoji]);
        let emoji = emoji_len();
        for (family, face) in [
            (BundledFamily::Sans, emoji),
            (BundledFamily::Serif, emoji),
            (BundledFamily::Mono, face_len("DejaVuSansMono.ttf")),
        ] {
            let glyphs = shaped(&mut font_cx, "\u{2764}", |b| {
                b.push_family(family, Span::from_usize_range(0..3));
            });
            assert_eq!(glyphs.len(), 1, "{family:?}: {glyphs:?}");
            assert_eq!(glyphs[0].face, face, "{family:?}: {glyphs:?}");
            assert_ne!(glyphs[0].glyph, 0, "{family:?}: {glyphs:?}");
        }
    }

    /// Before the emoji face arrives, the generic emoji family is empty. The
    /// sequence shapes in its base family, as it did before the fix. Its base
    /// character arms the fetch.
    #[test]
    fn before_the_emoji_face_arrives_a_sequence_shapes_in_its_base_family() {
        let mut font_cx = context_with_all(&[]);
        let serif = face_len("DejaVuSerif.ttf");
        let text = "\u{263A}\u{FE0F}";
        let glyphs = shaped(&mut font_cx, text, |b| {
            b.push_family(BundledFamily::Serif, Span::from_usize_range(0..text.len()));
        });
        assert!(!glyphs.is_empty());
        assert!(glyphs.iter().all(|g| g.face == serif), "{glyphs:?}");
        assert_eq!(LazyFont::for_char('\u{263A}'), Some(LazyFont::Emoji));
    }

    /// The letters around a sequence keep the family of their span, and the
    /// sequence takes that family as its base.
    #[test]
    fn the_letters_around_a_sequence_keep_their_family() {
        let mut font_cx = context_with_all(&[LazyFont::Emoji]);
        let (serif, emoji) = (face_len("DejaVuSerif.ttf"), emoji_len());
        let text = "a\u{2764}\u{FE0F}b";
        let glyphs = shaped(&mut font_cx, text, |b| {
            b.push_family(BundledFamily::Serif, Span::from_usize_range(0..text.len()));
        });
        let faces: Vec<usize> = glyphs.iter().map(|g| g.face).collect();
        assert_eq!(faces, vec![serif, emoji, serif], "{glyphs:?}");
    }

    /// A family pushed as a plain property is dropped, so the family of
    /// every stretch is one the builder knows (`CANVAS-090`).
    #[test]
    fn a_family_pushed_as_a_property_is_dropped() {
        let mut font_cx = context_with_all(&[]);
        let sans = face_len("NotoSans-Regular.ttf");
        let mono = FontFamily::named(BundledFamily::Mono.face_name());
        let glyphs = shaped(&mut font_cx, "ab", |b| {
            b.push_default(StyleProperty::FontFamily(mono.clone()));
            b.push(
                StyleProperty::FontFamily(mono),
                Span::from_usize_range(0..2),
            );
        });
        assert!(glyphs.iter().all(|g| g.face == sans), "{glyphs:?}");
    }

    fn sequences(text: &str) -> Vec<core::ops::Range<usize>> {
        emoji_sequences(text)
            .map(Span::<Byte>::to_usize_range)
            .collect()
    }

    /// Each sequence spans its base, its modifiers and each joined character.
    /// A sequence with no presentation selector is left to the fallback table.
    #[test]
    fn a_sequence_spans_its_base_and_what_follows_it() {
        assert_eq!(sequences("\u{2764}\u{FE0F}"), vec![0..6]);
        assert_eq!(sequences("a\u{2764}\u{FE0F}b"), vec![1..7]);
        // A keycap: the digit, the selector and the enclosing keycap.
        assert_eq!(sequences("1\u{FE0F}\u{20E3}"), vec![0..7]);
        // A flag of four characters, joined.
        assert_eq!(sequences("\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}"), vec![0..14]);
        // A man with a beard: the selector closes the joined character.
        assert_eq!(sequences("\u{1F9D4}\u{200D}\u{2642}\u{FE0F}"), vec![0..13]);
        // A skin tone, then a second sequence.
        assert_eq!(
            sequences("\u{270C}\u{1F3FD}\u{FE0F}\u{2764}\u{FE0F}"),
            vec![0..10, 10..16]
        );
        assert!(sequences("\u{2764}").is_empty());
        assert!(sequences("\u{1F469}\u{200D}\u{1F4BB}").is_empty());
        assert!(sequences("plain text").is_empty());
        assert!(sequences("\u{FE0F}").is_empty());
        assert!(holds_emoji_sequence("ok \u{263A}\u{FE0F}"));
        assert!(!holds_emoji_sequence("ok \u{263A}"));
    }
}
