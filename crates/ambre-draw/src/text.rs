// ambre-draw/src/text.rs
// ============================================================================
// The font choices every thread has to make the same way.
//
// Two peers paginate the same document and must reach the same page count
// (`features/ambre-spec-prose-pagination.md section 1.5`). A layout is a
// function of the text, the page box and the faces it was measured on. The
// first two are shared through the CRDT; the faces are not.
//
// So the faces are pinned instead. Ambre ships its own, none is taken from the
// device, and a family label is resolved by the label alone. What remains is
// script coverage, which a peer gains as it fetches. A set of `LazyScripts`
// names what a measurement was missing, so a consumer tells a settled layout
// from one still waiting for a face.
// ============================================================================

use ambre_core::{AssetPaths, Flags, FlagsKind, FontFamily, StaticKey};

use crate::face::FontFace;
use parley::fontique::Script;

/// One of the families the application ships (`PROSE-108`, `CANVAS-017`).
///
/// A document writes a family label as free text and nothing checks it. The
/// set of faces that label can reach is closed, and it is this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BundledFamily {
    /// Noto Sans, in regular, bold, italic and bold-italic.
    Sans,
    /// DejaVu Serif.
    Serif,
    /// DejaVu Sans Mono.
    Mono,
}

impl BundledFamily {
    /// The name the shaper knows this family by.
    #[must_use]
    pub fn face_name(self) -> &'static str {
        match self {
            BundledFamily::Sans => "Noto Sans",
            BundledFamily::Serif => "DejaVu Serif",
            BundledFamily::Mono => "DejaVu Sans Mono",
        }
    }
}

/// Resolve a family to a shipped face, reading nothing but the family.
///
/// This is the whole of family resolution in the workspace (`CANVAS-017`). It
/// never consults a font context, so two peers resolve one family alike even
/// when their contexts hold different faces (`PROSE-108`).
///
/// A face name reads as its family ([`FontFamily::from_label`]), which makes
/// the mapping idempotent: a caller may route an already-resolved face through
/// here again. A family no face ships for lands on the shipped sans. An
/// unmatched name handed to the shaper would produce no glyphs at all, and
/// that measures as an empty block.
#[must_use]
pub fn resolve_family(family: &FontFamily) -> BundledFamily {
    match family {
        FontFamily::Serif => BundledFamily::Serif,
        FontFamily::Mono => BundledFamily::Mono,
        FontFamily::Sans | FontFamily::Other(_) => BundledFamily::Sans,
    }
}

/// A script the shipped base faces do not cover, fetched when text needs it.
///
/// The base faces cover Latin, Latin Extended, Cyrillic and Greek. Anything
/// else is one of these, and it arrives only once a codepoint asks for it
/// (`CANVAS-006`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LazyFont {
    /// Han ideographs, Kana, Hangul, CJK symbols and full/half-width forms.
    Cjk,
    /// Arabic, its supplements, and the Arabic presentation forms.
    Arabic,
    /// Devanagari.
    Devanagari,
    /// Full-color emoji, from the CBDT bitmap build of Noto Color Emoji.
    Emoji,
}

impl LazyFont {
    /// Every lazy script, in the order [`LazyScripts::iter`] reports them.
    pub const ALL: [LazyFont; 4] = [
        LazyFont::Cjk,
        LazyFont::Arabic,
        LazyFont::Devanagari,
        LazyFont::Emoji,
    ];

    /// Classify a codepoint into the lazy script covering it.
    ///
    /// `None` means the base faces already cover it: Latin, Cyrillic, Greek,
    /// punctuation and symbols. Only a `Some` can ever ask for a fetch.
    ///
    /// The ranges follow the Unicode blocks, and they take the common blocks
    /// per script rather than every one. An unassigned codepoint inside a
    /// covered block maps to that script, which still renders tofu from the
    /// right family.
    #[must_use]
    pub fn for_char(c: char) -> Option<LazyFont> {
        match c as u32 {
            // Arabic, Arabic Supplement, Arabic Extended-A, and the two
            // presentation-forms blocks.
            0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF => Some(LazyFont::Arabic),
            // Devanagari.
            0x0900..=0x097F => Some(LazyFont::Devanagari),
            // CJK: symbols and punctuation, Hiragana, Katakana, Han ext-A, Han
            // unified, Hangul syllables, CJK compatibility ideographs, and the
            // half and full-width forms.
            0x3000..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF => Some(LazyFont::Cjk),
            // Emoji: the supplementary-plane pictograph blocks, the enclosed
            // and symbol blocks including the regional indicators, plus the
            // BMP Miscellaneous Symbols and Dingbats.
            //
            // The BMP text-arrows block is left to the base faces. Those
            // glyphs are monochrome symbols, so claiming them here would fetch
            // the emoji face and still tofu.
            0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF => Some(LazyFont::Emoji),
            _ => None,
        }
    }

    /// The family name this script's face registers under.
    ///
    /// A collection that answers this name holds the face, so a caller can ask
    /// whether it is able to measure the script at all (`PROSE-110`).
    #[must_use]
    pub fn face_name(self) -> &'static str {
        match self {
            LazyFont::Cjk => "Noto Sans SC",
            LazyFont::Arabic => "Noto Sans Arabic",
            LazyFont::Devanagari => "Noto Sans Devanagari",
            LazyFont::Emoji => "Noto Color Emoji",
        }
    }

    /// The scripts, as the shaper itemizes text, whose runs fall back to this
    /// face when the asked family misses a glyph (`CANVAS-089`).
    ///
    /// A CJK punctuation mark and some emoji itemize as the common script, so
    /// both faces answer it. Parley reaches most emoji through the generic
    /// emoji family instead, which [`register_face`] also wires.
    #[must_use]
    pub fn fallback_scripts(self) -> &'static [Script] {
        const CJK: [Script; 6] = [
            Script::from_bytes(*b"Hani"),
            Script::from_bytes(*b"Hira"),
            Script::from_bytes(*b"Kana"),
            Script::from_bytes(*b"Hang"),
            Script::from_bytes(*b"Bopo"),
            COMMON_SCRIPT,
        ];
        const ARABIC: [Script; 1] = [Script::from_bytes(*b"Arab")];
        const DEVANAGARI: [Script; 1] = [Script::from_bytes(*b"Deva")];
        const EMOJI: [Script; 1] = [COMMON_SCRIPT];
        match self {
            LazyFont::Cjk => &CJK,
            LazyFont::Arabic => &ARABIC,
            LazyFont::Devanagari => &DEVANAGARI,
            LazyFont::Emoji => &EMOJI,
        }
    }

    /// The same-origin asset this script's face is fetched from.
    ///
    /// The files live under `crates/ambre-app/fonts/` and Trunk copies them to
    /// `dist/fonts/`. A missing file leaves tofu rather than failing a boot.
    #[must_use]
    pub fn asset_url(self) -> StaticKey<AssetPaths> {
        StaticKey::new(match self {
            LazyFont::Cjk => "/fonts/NotoSansSC-Regular.otf",
            LazyFont::Arabic => "/fonts/NotoSansArabic-Regular.ttf",
            LazyFont::Devanagari => "/fonts/NotoSansDevanagari-Regular.ttf",
            LazyFont::Emoji => "/fonts/NotoColorEmoji.ttf",
        })
    }

    /// The set that holds this script alone: its bit in a set of
    /// [`LazyScripts`].
    #[must_use]
    pub const fn as_set(self) -> Flags<LazyScripts> {
        match self {
            LazyFont::Cjk => Flags::of::<1>(),
            LazyFont::Arabic => Flags::of::<2>(),
            LazyFont::Devanagari => Flags::of::<4>(),
            LazyFont::Emoji => Flags::of::<8>(),
        }
    }
}

/// The lazy scripts, the marker of the set `Flags<LazyScripts>`: held as
/// bits so two peers compare one value. Each member is a
/// [`LazyFont::as_set`].
///
/// Pagination carries one of these to name the scripts a document used and no
/// face covered (`PROSE-110`). Empty means the measurement had every face it
/// asked for, which is the only state a stored page number may rest on
/// (`PROSE-112`).
#[derive(PartialEq, Eq)]
pub enum LazyScripts {}

impl FlagsKind for LazyScripts {
    type Repr = u8;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ScriptSet");
    const ALL: u8 = 0b1111;
}

impl LazyScripts {
    /// The scripts in `set`, in the order of [`LazyFont::ALL`].
    pub fn iter(set: Flags<Self>) -> impl Iterator<Item = LazyFont> {
        LazyFont::ALL
            .into_iter()
            .filter(move |script| set.contains(script.as_set()))
    }

    /// Every lazy script a string asks for.
    #[must_use]
    pub fn of_text(text: &str) -> Flags<Self> {
        text.chars()
            .filter_map(LazyFont::for_char)
            .fold(Flags::EMPTY, |set, script| set.with(script.as_set()))
    }
}

/// A font context holding nothing but what a caller registers into it
/// (`PROSE-114`, `PDF-007`, `CANVAS-069`).
///
/// A default context discovers the faces of the machine it runs on. Parley
/// then falls back to them for any codepoint the asked family misses, inside
/// the shaper and below every rule a caller states. A device face would bend a
/// measurement or a page with nothing to catch it. Sealing the collection is
/// the only place the ban can hold, so every thread builds its context here.
#[must_use]
pub fn sealed_font_context() -> parley::FontContext {
    parley::FontContext {
        collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        source_cache: parley::fontique::SourceCache::default(),
    }
}

/// The script of a codepoint every script shares: punctuation, symbols, most
/// emoji.
const COMMON_SCRIPT: Script = Script::from_bytes(*b"Zyyy");

/// The order in which the lazy faces answer a script two of them cover.
///
/// The emoji face comes first: the CJK face draws some emoji in monochrome.
/// The order is fixed, never the order of arrival. Two peers then shape one
/// codepoint on one face, whichever face each fetched first (`PROSE-108`).
const FALLBACK_ORDER: [LazyFont; 4] = [
    LazyFont::Emoji,
    LazyFont::Cjk,
    LazyFont::Arabic,
    LazyFont::Devanagari,
];

/// Register the faces of one font file into `font_cx`, and make each lazy
/// face the fallback of the scripts it covers (`CANVAS-089`).
///
/// A sealed context has no fallback table: the system backend alone fills
/// one, and the seal turns that backend off. Parley then tries the asked
/// family and nothing else, so a lazy script under a base family shapes as
/// glyph 0. Every registration of the workspace goes through here, so each
/// thread and the export context wire the same table.
///
/// The table names only the faces already in the collection, so a lazy face
/// still costs nothing until its script appears (`CANVAS-006`). Each call
/// rewrites the table whole, in [`FALLBACK_ORDER`]: a face registered twice
/// is named once, and the order of arrival leaves no trace.
///
/// The context registers the shared blob of `face`, so it reads the bytes the
/// caller holds and copies none (`CANVAS-097`).
pub fn register_face(font_cx: &mut parley::FontContext, face: &FontFace) {
    let families = face.register_into(&mut font_cx.collection);
    let held: Vec<(LazyFont, parley::fontique::FamilyId)> = FALLBACK_ORDER
        .into_iter()
        .filter_map(|script| {
            let id = font_cx.collection.family_id(script.face_name())?;
            Some((script, id))
        })
        .collect();
    let arrived = held
        .iter()
        .any(|(_, id)| families.iter().any(|(registered, _)| registered == id));
    if !arrived {
        // A base face: no lazy family arrived, so the table stays as it was.
        return;
    }
    for &(script, _) in &held {
        for &key in script.fallback_scripts() {
            let answering = held
                .iter()
                .filter(|(other, _)| other.fallback_scripts().contains(&key))
                .map(|&(_, id)| id);
            font_cx.collection.set_fallbacks(key, answering);
        }
    }
    if let Some(&(_, emoji)) = held.iter().find(|(script, _)| *script == LazyFont::Emoji) {
        // Parley asks the generic emoji family for a cluster it reads as an
        // emoji, before any script fallback.
        font_cx
            .collection
            .set_generic_families(parley::GenericFamily::Emoji, core::iter::once(emoji));
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::FontFamily;

    use ambre_core::Flags;

    use super::{BundledFamily, LazyFont, LazyScripts, resolve_family, sealed_font_context};

    fn resolve_label(label: &str) -> BundledFamily {
        resolve_family(&FontFamily::from_label(label))
    }

    #[test]
    fn a_face_name_resolves_to_itself() {
        for family in [
            BundledFamily::Sans,
            BundledFamily::Serif,
            BundledFamily::Mono,
        ] {
            assert_eq!(resolve_label(family.face_name()), family);
        }
    }

    #[test]
    fn an_unknown_label_lands_on_the_shipped_sans() {
        assert_eq!(resolve_label("Comic Sans MS"), BundledFamily::Sans);
        assert_eq!(resolve_family(&FontFamily::default()), BundledFamily::Sans);
    }

    #[test]
    fn the_toolbar_labels_reach_their_face() {
        assert_eq!(resolve_label("Serif"), BundledFamily::Serif);
        assert_eq!(resolve_label("Monospace"), BundledFamily::Mono);
        assert_eq!(resolve_label("Sans"), BundledFamily::Sans);
    }

    #[test]
    fn latin_and_punctuation_ask_for_no_face() {
        let set = LazyScripts::of_text("Hello, world -- 123 (ok)? Grec: alpha.");
        assert!(set.is_empty());
    }

    #[test]
    fn a_script_outside_the_base_faces_is_named() {
        let set = LazyScripts::of_text("hello");
        assert!(set.is_empty());
        let cjk = LazyScripts::of_text("\u{4e00}");
        assert!(cjk.contains(LazyFont::Cjk.as_set()));
        assert!(!cjk.contains(LazyFont::Arabic.as_set()));
    }

    #[test]
    fn a_set_reports_its_scripts_in_one_order() {
        let mut set = Flags::<LazyScripts>::EMPTY;
        set.insert(LazyFont::Emoji.as_set());
        set.insert(LazyFont::Cjk.as_set());
        let seen: Vec<LazyFont> = LazyScripts::iter(set).collect();
        assert_eq!(seen, vec![LazyFont::Cjk, LazyFont::Emoji]);
        assert_eq!(format!("{set:?}"), "ScriptSet(9)");
    }

    #[test]
    fn a_difference_drops_what_the_other_holds() {
        let asked = LazyScripts::of_text("\u{4e00}\u{0627}");
        let held = LazyFont::Cjk.as_set();
        let missing = asked.without(held);
        assert!(missing.contains(LazyFont::Arabic.as_set()));
        assert!(!missing.contains(LazyFont::Cjk.as_set()));
    }

    /// A sealed context holds nothing the machine happens to carry
    /// (`PROSE-114`, `PDF-007`, `CANVAS-069`).
    ///
    /// The families below ship with common desktop distributions and none is
    /// one of Ambre's own faces. A default context finds them where they exist,
    /// so a sealed one finding none is the ban holding.
    #[test]
    fn a_sealed_context_discovers_no_face_of_the_machine() {
        let mut sealed = sealed_font_context();
        for family in [
            "DejaVu Sans",
            "Liberation Sans",
            "FreeSans",
            "Bitstream Charter",
            "IPAGothic",
            "Arial",
            "Helvetica",
        ] {
            assert!(
                sealed.collection.family_id(family).is_none(),
                "a sealed context found `{family}`"
            );
        }
    }
}

/// Each lazy face, once registered, serves its script under every base family
/// (`CANVAS-089`).
///
/// A caller names a base family and nothing else. The shaper then has to find
/// the lazy face by itself, on a sealed context, as a peer shapes a document
/// that mixes scripts. Glyph 0 is the face's missing glyph: a run that holds
/// one drew tofu.
#[cfg(test)]
mod fallback_tests {
    use ambre_core::Payload;
    use parley::{
        FontContext, FontFamily, FontFamilyName, LayoutContext, PositionedLayoutItem, StyleProperty,
    };

    use super::{BundledFamily, LazyFont, register_face, sealed_font_context};
    use crate::ParleyBrush;
    use crate::face::FontFace;

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");

    /// The base faces, as `ambre-canvas` fetches them at boot.
    const BASE: [&str; 6] = [
        "NotoSans-Regular.ttf",
        "NotoSans-Bold.ttf",
        "NotoSans-Italic.ttf",
        "NotoSans-BoldItalic.ttf",
        "DejaVuSerif.ttf",
        "DejaVuSansMono.ttf",
    ];

    fn read(file: &str) -> Payload<ambre_core::FontBytes> {
        let path = format!("{DIR}{file}");
        Payload::new(std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
    }

    /// A sealed context fed the base faces, then the faces of `lazy`, in
    /// that order.
    fn context_with_all(lazy: &[LazyFont]) -> FontContext {
        let mut font_cx = sealed_font_context();
        for file in BASE {
            register_face(&mut font_cx, &FontFace::new(read(file)));
        }
        for script in lazy {
            let url = script.asset_url();
            register_face(
                &mut font_cx,
                &FontFace::new(read(url.as_str().trim_start_matches("/fonts/"))),
            );
        }
        font_cx
    }

    /// A sealed context fed the base faces, then the face of `lazy` alone.
    fn context_with(lazy: LazyFont) -> FontContext {
        context_with_all(&[lazy])
    }

    /// The glyph ids `text` shapes to in `family`.
    fn glyphs(font_cx: &mut FontContext, text: &str, family: BundledFamily) -> Vec<u32> {
        let mut layout_cx: LayoutContext<ParleyBrush> = LayoutContext::new();
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::Single(
            FontFamilyName::Named(family.face_name().into()),
        )));
        builder.push_default(StyleProperty::FontSize(16.0));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        let mut ids = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    ids.extend(run.glyphs().map(|glyph| glyph.id));
                }
            }
        }
        ids
    }

    /// Shape each sample under each base family, and refuse a glyph 0.
    fn assert_no_tofu(lazy: LazyFont, samples: &[&str]) {
        let mut font_cx = context_with(lazy);
        for family in [
            BundledFamily::Sans,
            BundledFamily::Serif,
            BundledFamily::Mono,
        ] {
            for text in samples {
                let ids = glyphs(&mut font_cx, text, family);
                assert!(!ids.is_empty(), "{lazy:?}: `{text}` shaped nothing");
                assert!(
                    !ids.contains(&0),
                    "{lazy:?}: `{text}` drew tofu under {}: {ids:?}",
                    family.face_name()
                );
            }
        }
    }

    #[test]
    fn arabic_falls_back_to_its_face_under_every_base_family() {
        assert_no_tofu(
            LazyFont::Arabic,
            &[
                "\u{645}\u{631}\u{62d}\u{628}\u{627}",
                "ok \u{633}\u{644}\u{627}\u{645}",
            ],
        );
    }

    #[test]
    fn devanagari_falls_back_to_its_face_under_every_base_family() {
        assert_no_tofu(
            LazyFont::Devanagari,
            &[
                "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947}",
                "ok \u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940}",
            ],
        );
    }

    #[test]
    fn cjk_falls_back_to_its_face_under_every_base_family() {
        assert_no_tofu(
            LazyFont::Cjk,
            &[
                "\u{4e2d}\u{6587}",
                "\u{304b}\u{306a}\u{30ab}\u{30ca}",
                "ok \u{6f22}\u{5b57}\u{3002}",
                "\u{300c}\u{3002}\u{300d}",
            ],
        );
    }

    #[test]
    fn emoji_falls_back_to_its_face_under_every_base_family() {
        assert_no_tofu(
            LazyFont::Emoji,
            &[
                "\u{1F44D}",
                "ok \u{1F44D}",
                "\u{2764}",
                "\u{2B50}",
                "\u{1F1EB}\u{1F1F7}",
            ],
        );
    }

    /// The faces that answer the common script, in the order the table
    /// holds them, by the lazy script each one serves.
    fn common_fallbacks(font_cx: &mut FontContext) -> Vec<LazyFont> {
        let key = parley::fontique::FallbackKey::new(super::COMMON_SCRIPT, None);
        let ids: Vec<_> = font_cx.collection.fallback_families(key).collect();
        ids.into_iter()
            .filter_map(|id| {
                LazyFont::ALL
                    .into_iter()
                    .find(|script| font_cx.collection.family_id(script.face_name()) == Some(id))
            })
            .collect()
    }

    /// Two peers that fetched the CJK and emoji faces in either order hold
    /// one fallback table (`CANVAS-089`).
    ///
    /// Both faces answer the common script. A table in order of arrival would
    /// shape a symbol both faces draw on a different face per peer. The two
    /// peers would then paginate apart.
    #[test]
    fn the_order_of_arrival_leaves_the_fallback_table_unchanged() {
        let mut cjk_first = context_with_all(&[LazyFont::Cjk, LazyFont::Emoji]);
        let mut emoji_first = context_with_all(&[LazyFont::Emoji, LazyFont::Cjk]);
        let expected = vec![LazyFont::Emoji, LazyFont::Cjk];
        assert_eq!(common_fallbacks(&mut cjk_first), expected);
        assert_eq!(common_fallbacks(&mut emoji_first), expected);

        // A face delivered twice is named once.
        let mut twice = context_with_all(&[LazyFont::Cjk, LazyFont::Cjk]);
        assert_eq!(common_fallbacks(&mut twice), vec![LazyFont::Cjk]);
    }
}
