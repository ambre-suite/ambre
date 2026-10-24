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
// script coverage, which a peer gains as it fetches. `ScriptSet` names what a
// measurement was missing, so a consumer tells a settled layout from one
// still waiting for a face.
// ============================================================================

use ambre_core::FontFamily;

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
/// here again. A family no face ships for lands on the shipped sans, because an
/// unmatched name handed to the shaper would produce no glyphs at all, which
/// measures as an empty block.
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
    /// Every lazy script, in the order [`ScriptSet`] reports them.
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

    /// The same-origin asset this script's face is fetched from.
    ///
    /// The files live under `crates/ambre-app/fonts/` and Trunk copies them to
    /// `dist/fonts/`. A missing file leaves tofu rather than failing a boot.
    #[must_use]
    pub fn asset_url(self) -> &'static str {
        match self {
            LazyFont::Cjk => "/fonts/NotoSansSC-Regular.otf",
            LazyFont::Arabic => "/fonts/NotoSansArabic-Regular.ttf",
            LazyFont::Devanagari => "/fonts/NotoSansDevanagari-Regular.ttf",
            LazyFont::Emoji => "/fonts/NotoColorEmoji.ttf",
        }
    }

    /// The bit this script holds in a [`ScriptSet`].
    const fn bit(self) -> u8 {
        match self {
            LazyFont::Cjk => 1,
            LazyFont::Arabic => 2,
            LazyFont::Devanagari => 4,
            LazyFont::Emoji => 8,
        }
    }
}

/// A set of lazy scripts, held as bits so two peers compare one value.
///
/// Pagination carries one of these to name the scripts a document used and no
/// face covered (`PROSE-110`). Empty means the measurement had every face it
/// asked for, which is the only state a stored page number may rest on
/// (`PROSE-112`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptSet(u8);

impl ScriptSet {
    /// The empty set: nothing was missing.
    #[must_use]
    pub const fn new() -> Self {
        Self(0)
    }

    /// Add a script to the set.
    pub fn insert(&mut self, script: LazyFont) {
        self.0 |= script.bit();
    }

    /// Whether the set holds this script.
    #[must_use]
    pub const fn contains(self, script: LazyFont) -> bool {
        self.0 & script.bit() != 0
    }

    /// Whether the set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The scripts in the set, in the order of [`LazyFont::ALL`].
    pub fn iter(self) -> impl Iterator<Item = LazyFont> {
        LazyFont::ALL
            .into_iter()
            .filter(move |script| self.contains(*script))
    }

    /// Every lazy script a string asks for.
    #[must_use]
    pub fn of_text(text: &str) -> Self {
        let mut set = Self::new();
        for c in text.chars() {
            if let Some(script) = LazyFont::for_char(c) {
                set.insert(script);
            }
        }
        set
    }

    /// The scripts in `self` that `other` does not hold.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Fold another set into this one.
    pub fn extend(&mut self, other: Self) {
        self.0 |= other.0;
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

#[cfg(test)]
mod tests {
    use ambre_core::FontFamily;

    use super::{BundledFamily, LazyFont, ScriptSet, resolve_family, sealed_font_context};

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
        let set = ScriptSet::of_text("Hello, world -- 123 (ok)? Grec: alpha.");
        assert!(set.is_empty());
    }

    #[test]
    fn a_script_outside_the_base_faces_is_named() {
        let set = ScriptSet::of_text("hello");
        assert!(set.is_empty());
        let cjk = ScriptSet::of_text("\u{4e00}");
        assert!(cjk.contains(LazyFont::Cjk));
        assert!(!cjk.contains(LazyFont::Arabic));
    }

    #[test]
    fn a_set_reports_its_scripts_in_one_order() {
        let mut set = ScriptSet::new();
        set.insert(LazyFont::Emoji);
        set.insert(LazyFont::Cjk);
        let seen: Vec<LazyFont> = set.iter().collect();
        assert_eq!(seen, vec![LazyFont::Cjk, LazyFont::Emoji]);
    }

    #[test]
    fn a_difference_drops_what_the_other_holds() {
        let asked = ScriptSet::of_text("\u{4e00}\u{0627}");
        let mut held = ScriptSet::new();
        held.insert(LazyFont::Cjk);
        let missing = asked.difference(held);
        assert!(missing.contains(LazyFont::Arabic));
        assert!(!missing.contains(LazyFont::Cjk));
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
