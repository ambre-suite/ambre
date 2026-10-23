//! The inline mark registry: what a mark key means, how its value is spelled,
//! and what an adjacent insertion does to it
//! (`ambre-spec-prose-schema.md section 3`).
//!
//! [`expand_type_for`] is the one place a mark's expansion behaviour is decided
//! (`PROSE-007`). [`configure`] builds the document's whole style configuration
//! from it. A call site cannot pick a different policy for the same key,
//! because it never states one.

use ambre_bus::app::prose::MarkKey;
use ambre_core::{FontSize, Rgb, Rgba};
use loro::{ExpandType, LoroDoc, StyleConfig, StyleConfigMap};

ambre_core::crdt_names! {
    /// The roots the registry declares. A key is a root, or a root and an id
    /// joined by a colon, and Loro reads it under its root. A document may
    /// carry a mark only under one of these.
    pub enum MarkRoot {
        Bold = "b",
        Italic = "i",
        Underline = "u",
        Strike = "s",
        Superscript = "sup",
        Subscript = "sub",
        Code = "c",
        /// A link: formatting that the style path writes, and an anchor too.
        Link = "a",
        Ink = "fg",
        Highlight = "bg",
        FontSize = "fs",
        FontFamily = "ff",
        /// A comment thread. Each thread writes its own key under it,
        /// `cm:{id}`, so two threads over one character never share a value.
        Comment = "cm",
        /// The inserted text of a suggestion.
        Insertion = "si",
        /// The deleted text of a suggestion.
        Deletion = "sd",
        /// A footnote call: the note's identity in `annots`.
        Footnote = "fn",
        /// A cross reference: its record, in hex (`PROSE-FEAT-072`).
        CrossRef = "ref",
        /// A header field, addressed through its children `f:pn`, `f:pc`,
        /// `f:dt` and `f:ti`.
        Field = "f",
    }
}

impl MarkRoot {
    /// The root `key` is read under, or `None` for a root this build does not
    /// declare.
    #[must_use]
    pub fn of(key: &MarkKey) -> Option<Self> {
        Self::parse(key.root())
    }

    /// The key that is this root alone.
    #[must_use]
    pub fn key(self) -> MarkKey {
        MarkKey::from(self.as_str())
    }
}

/// The formatting keys: the mark's value is the formatting itself, and an
/// insertion at the edge of the range takes the formatting with it.
///
/// `ClearInlineMarks` removes exactly these. A comment, a suggestion, a
/// footnote and a cross reference are anchors rather than formatting, and each
/// has a feature that owns its removal.
pub const FORMATTING_KEYS: [MarkRoot; 12] = [
    MarkRoot::Bold,
    MarkRoot::Italic,
    MarkRoot::Underline,
    MarkRoot::Strike,
    MarkRoot::Superscript,
    MarkRoot::Subscript,
    MarkRoot::Code,
    MarkRoot::Link,
    MarkRoot::Ink,
    MarkRoot::Highlight,
    MarkRoot::FontSize,
    MarkRoot::FontFamily,
];

/// The keys whose value is an identifier or a record, and which therefore must
/// never swallow an adjacent insertion.
///
/// Loro reads a key of the shape `a:b` under its root `a`. The one entry `f`
/// therefore covers `f:pn`, `f:pc`, `f:dt` and `f:ti`.
const ANCHOR_KEYS: [MarkRoot; 7] = [
    MarkRoot::Link,
    MarkRoot::Comment,
    MarkRoot::Insertion,
    MarkRoot::Deletion,
    MarkRoot::Footnote,
    MarkRoot::CrossRef,
    MarkRoot::Field,
];

/// The roots a command of their own owns: a comment, a suggestion, a
/// footnote, a cross reference and a header field. The link is an anchor too,
/// and the one the style path writes.
const OWNED_ROOTS: [MarkRoot; 6] = [
    MarkRoot::Comment,
    MarkRoot::Insertion,
    MarkRoot::Deletion,
    MarkRoot::Footnote,
    MarkRoot::CrossRef,
    MarkRoot::Field,
];

/// Whether a key belongs to an anchor its own command writes and removes, so
/// the style path must refuse it (`PROSE-FEAT-298`, `PROSE-FEAT-299`).
#[must_use]
pub fn is_owned_anchor(mark_key: &MarkKey) -> bool {
    MarkRoot::of(mark_key).is_some_and(|root| OWNED_ROOTS.contains(&root))
}

/// The expansion an insertion at the edge of a mark gets.
///
/// Total over every key (`PROSE-090`): a key no build knows answers the
/// default, which is what a formatting mark does. A panic here would kill the
/// worker over a document written by a newer client.
#[must_use]
pub fn expand_type_for(mark_key: &MarkKey) -> ExpandType {
    // A key of the shape `a:b` belongs to the registry entry `a`, which is how
    // Loro itself resolves one.
    MarkRoot::of(mark_key).map_or(ExpandType::After, expand_of)
}

/// The expansion of a mark under `root`.
fn expand_of(root: MarkRoot) -> ExpandType {
    if ANCHOR_KEYS.contains(&root) {
        ExpandType::None
    } else {
        ExpandType::After
    }
}

/// Whether a command may write a mark under this key.
///
/// The CRDT engine holds a configuration per registered root and refuses every
/// other key. A command that names one is therefore refused rather than
/// silently dropped. The check sits here, so the refusal names the registry
/// rather than the engine.
#[must_use]
pub fn is_registered(mark_key: &MarkKey) -> bool {
    MarkRoot::of(mark_key).is_some()
}

/// Teach a document the expansion policy of every registered key
/// (`PROSE-007`).
///
/// Called once, at construction, before any mark is written. Every policy comes
/// from [`expand_type_for`], so the registry has one author.
///
/// Each key is registered under its own root, because `StyleConfigMap` refuses
/// a key holding the separator and resolves `a:b` under `a` anyway.
pub(crate) fn configure(doc: &LoroDoc) {
    let mut styles = StyleConfigMap::new();
    for &root in MarkRoot::ALL {
        styles.insert(
            root.as_str().into(),
            StyleConfig {
                expand: expand_of(root),
            },
        );
    }
    doc.config_text_style(styles);
}

/// The canonical spelling of a number carried as a mark value (`PROSE-008`).
///
/// Base 10, a `.` separator, no exponent, no trailing zero and no unit. Two
/// peers writing the same size therefore produce the same bytes, and the merge
/// holds one value rather than a choice between two spellings (`PROSE-091`).
///
/// A value that is not finite has no canonical spelling, and the caller refuses
/// the command rather than writing a marker no reader can use.
#[must_use]
pub fn canonical_number(value: f32) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    // `f32::to_string` prints the shortest decimal that reads back as the same
    // value, with no trailing zero. It prints an exponent outside the range a
    // font size or a colour component lives in, and the caller refuses that.
    let rendered = value.to_string();
    (!rendered.contains(['e', 'E'])).then_some(rendered)
}

/// The keys whose presence alone is the formatting, and which therefore take
/// no value (`ambre-spec-prose-schema.md section 3`).
const FLAG_KEYS: [MarkRoot; 7] = [
    MarkRoot::Bold,
    MarkRoot::Italic,
    MarkRoot::Underline,
    MarkRoot::Strike,
    MarkRoot::Superscript,
    MarkRoot::Subscript,
    MarkRoot::Code,
];

/// The keys whose value is a colour.
const COLOR_KEYS: [MarkRoot; 2] = [MarkRoot::Ink, MarkRoot::Highlight];

/// The smallest font size a mark may carry.
const FONT_SIZE_MIN: FontSize = FontSize::from_whole_points(1);
/// The largest font size the office formats round-trip.
const FONT_SIZE_MAX: FontSize = FontSize::from_whole_points(1638);

/// Why a mark value was refused before the document was touched
/// (`PROSE-FEAT-022`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkValueError {
    /// A colour that is neither six nor eight hexadecimal digits behind a hash.
    BadColor(String),
    /// A size that is not finite, or outside one to 1638 points.
    BadFontSize(String),
    /// A value supplied for a key that takes none, or missing for a key that
    /// requires one.
    ArityMismatch { key: MarkKey },
    /// A key the registry does not declare.
    UnknownKey(MarkKey),
}

/// Parse a colour into red, green, blue and alpha, packed in that order.
///
/// The six-digit form is opaque. The two spellings of one opaque colour
/// therefore parse to one value (`PROSE-FEAT-175`), and a round trip of the
/// stored bytes is stable (`PROSE-FEAT-023`).
///
/// # Errors
/// [`MarkValueError::BadColor`] for anything but a hash and six or eight
/// hexadecimal digits.
pub fn parse_color(raw: &str) -> Result<Rgba, MarkValueError> {
    let bad = || MarkValueError::BadColor(raw.to_owned());
    let digits = raw.strip_prefix('#').ok_or_else(bad)?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16)
            .map(|rgb| Rgb::from_bits(rgb).to_rgba(0xFF))
            .map_err(|_| bad()),
        8 => u32::from_str_radix(digits, 16)
            .map(Rgba::from_bits)
            .map_err(|_| bad()),
        _ => Err(bad()),
    }
}

/// The one spelling a parsed colour is stored as (`PROSE-FEAT-175`).
///
/// A fully opaque colour is spelled in six digits, and every other in eight.
/// Lower case throughout, so two peers writing one colour write one string.
#[must_use]
pub fn canonical_color(rgba: Rgba) -> String {
    match rgba.channels() {
        [_, _, _, 0xFF] => format!("#{:06x}", rgba.rgb().bits()),
        _ => format!("#{:08x}", rgba.bits()),
    }
}

/// Parse a size in points. The ceiling is the one the office formats
/// round-trip.
///
/// # Errors
/// [`MarkValueError::BadFontSize`] for a value that does not read as a number,
/// is not finite, or falls outside one to 1638 points.
pub fn parse_font_size(raw: &str) -> Result<FontSize, MarkValueError> {
    let bad = || MarkValueError::BadFontSize(raw.to_owned());
    let points = raw.parse::<f32>().map_err(|_| bad())?;
    FontSize::new(points)
        .filter(|size| (FONT_SIZE_MIN..=FONT_SIZE_MAX).contains(size))
        .ok_or_else(bad)
}

/// The value a command may write under `key`, in its canonical spelling, or the
/// refusal the author is told about (`PROSE-FEAT-022`, `PROSE-FEAT-290`).
///
/// The grammar is read from the key and checked before the document is touched.
/// A key whose presence alone is the formatting stores the empty string, which
/// is the one spelling of "present".
///
/// # Errors
/// One [`MarkValueError`] per way the pair can fail. An unknown key, a value
/// where none belongs, none where one is required, a malformed colour, and a
/// size outside the range.
pub fn validate_value(key: &MarkKey, raw: Option<&str>) -> Result<String, MarkValueError> {
    let Some(root) = MarkRoot::of(key) else {
        return Err(MarkValueError::UnknownKey(key.clone()));
    };
    let arity = || MarkValueError::ArityMismatch { key: key.clone() };
    if FLAG_KEYS.contains(&root) {
        return match raw {
            None | Some("") => Ok(String::new()),
            Some(_) => Err(arity()),
        };
    }
    let raw = raw.ok_or_else(arity)?;
    if COLOR_KEYS.contains(&root) {
        return Ok(canonical_color(parse_color(raw)?));
    }
    if NUMERIC_KEYS.contains(&root) {
        let size = parse_font_size(raw)?;
        return canonical_number(size.points())
            .ok_or_else(|| MarkValueError::BadFontSize(raw.to_owned()));
    }
    Ok(raw.to_owned())
}

/// The keys whose value is a number, and the only ones a re-spelling applies to.
///
/// Every other key of section 3 carries text: a URL, a family name, a `Uuid`,
/// a hex record, a `#rrggbb` colour. The float parser would rewrite those. A
/// cross reference of `"1e5"` reads as a number, and a footnote of `"007"`
/// loses its zeroes. So the grammar is read from the key.
const NUMERIC_KEYS: [MarkRoot; 1] = [MarkRoot::FontSize];

/// The canonical spelling of a mark value a command carried, or `None` when the
/// value has none (`PROSE-008`, `PROSE-091`).
///
/// A value under a numeric key is re-spelled through [`canonical_number`], so
/// `"11.0"` and `"11"` reach the document as one string. A value that is not a
/// finite number is refused. Every other key's value is its own canonical form:
/// a URL, a family name and a `Uuid` are spelled once.
#[must_use]
pub fn canonical_value(mark_key: &MarkKey, raw: &str) -> Option<String> {
    if !MarkRoot::of(mark_key).is_some_and(|root| NUMERIC_KEYS.contains(&root)) {
        return Some(raw.to_owned());
    }
    canonical_number(raw.parse::<f32>().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_formatting_mark_swallows_an_insertion_at_its_edge() {
        for key in [
            "b", "i", "u", "s", "sup", "sub", "c", "fg", "bg", "fs", "ff",
        ] {
            assert_eq!(expand_type_for(&key.into()), ExpandType::After, "{key}");
        }
    }

    #[test]
    fn an_anchor_mark_never_swallows_an_insertion() {
        for root in ANCHOR_KEYS {
            assert_eq!(expand_type_for(&root.key()), ExpandType::None, "{root:?}");
        }
    }

    #[test]
    fn a_header_field_is_read_under_its_own_root() {
        for key in ["f:pn", "f:pc", "f:dt", "f:ti"] {
            assert_eq!(expand_type_for(&key.into()), ExpandType::None, "{key}");
            assert!(is_registered(&key.into()), "{key}");
        }
    }

    #[test]
    fn an_unknown_key_answers_the_default_and_never_panics() {
        assert_eq!(
            expand_type_for(&"a-key-no-build-knows".into()),
            ExpandType::After
        );
        assert_eq!(expand_type_for(&"".into()), ExpandType::After);
        assert_eq!(expand_type_for(&":".into()), ExpandType::After);
        assert!(!is_registered(&"a-key-no-build-knows".into()));
    }

    #[test]
    fn two_peers_spell_one_size_the_same_way() {
        assert_eq!(canonical_value(&"fs".into(), "11.0").as_deref(), Some("11"));
        assert_eq!(canonical_value(&"fs".into(), "11").as_deref(), Some("11"));
        assert_eq!(
            canonical_value(&"fs".into(), "11.50").as_deref(),
            Some("11.5")
        );
        assert_eq!(canonical_number(0.5).as_deref(), Some("0.5"));
        assert_eq!(canonical_number(f32::NAN), None);
        assert_eq!(canonical_number(f32::INFINITY), None);
    }

    #[test]
    fn a_value_that_is_not_a_number_is_its_own_canonical_form() {
        assert_eq!(
            canonical_value(&"a".into(), "https://example.org/a").as_deref(),
            Some("https://example.org/a")
        );
        assert_eq!(
            canonical_value(&"fg".into(), "#ff8800").as_deref(),
            Some("#ff8800")
        );
    }

    /// A value under a key the registry spells as text reaches the document as
    /// the user wrote it. Each of these reads as a number to the float parser,
    /// and re-spelling it would name another anchor or another font.
    #[test]
    fn a_textual_value_that_reads_as_a_number_is_left_alone() {
        assert_eq!(
            canonical_value(&"ref".into(), "1.10").as_deref(),
            Some("1.10")
        );
        assert_eq!(canonical_value(&"fn".into(), "007").as_deref(), Some("007"));
        assert_eq!(canonical_value(&"cm".into(), "1e5").as_deref(), Some("1e5"));
        assert_eq!(canonical_value(&"ff".into(), "inf").as_deref(), Some("inf"));
        assert_eq!(
            canonical_value(&"f:pn".into(), "NaN").as_deref(),
            Some("NaN")
        );
    }

    /// The numeric key is the one that refuses. A size with no finite value
    /// has no canonical spelling. The command is refused, rather than the
    /// document left holding a marker no reader can use.
    #[test]
    fn a_size_that_is_not_a_finite_number_has_no_canonical_spelling() {
        assert_eq!(canonical_value(&"fs".into(), "inf"), None);
        assert_eq!(canonical_value(&"fs".into(), "eleven"), None);
    }
}
