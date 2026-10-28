//! The value grammar of the inline mark registry: which roots a key may name,
//! and how the value of each root is spelled (`ambre-spec-prose-schema.md
//! section 3`).
//!
//! The grammar lives on the bus so that [`ProseCommand::ApplyMark`] carries a
//! [`MarkValue`] already checked, rather than the text the UI typed. The
//! expansion policy, which reads Loro, stays in `ambre-prose`.
//!
//! [`ProseCommand::ApplyMark`]: super::ProseCommand::ApplyMark

use super::MarkKey;
use ambre_core::{FontSize, Rgb, Rgba};

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
            .map(|rgb| Rgb::from_bits(rgb).to_rgba(ambre_core::Alpha::OPAQUE))
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

/// A mark value in the one spelling the grammar of its key allows. It is the
/// empty string of a flag, a canonical colour, a canonical number, or the text
/// of a key that carries text.
///
/// [`validate_value`] and [`canonical_value`] are the only ways in, so a value
/// of this type has passed the grammar of the key it was checked under. A
/// value read back from the document has not, and stays a string: a peer on
/// another build may have written it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MarkValue(String);

impl MarkValue {
    /// The value, for the call that writes it into the text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
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
pub fn validate_value(key: &MarkKey, raw: Option<&str>) -> Result<MarkValue, MarkValueError> {
    let Some(root) = MarkRoot::of(key) else {
        return Err(MarkValueError::UnknownKey(key.clone()));
    };
    let arity = || MarkValueError::ArityMismatch { key: key.clone() };
    if FLAG_KEYS.contains(&root) {
        return match raw {
            None | Some("") => Ok(MarkValue(String::new())),
            Some(_) => Err(arity()),
        };
    }
    let raw = raw.ok_or_else(arity)?;
    if COLOR_KEYS.contains(&root) {
        return Ok(MarkValue(canonical_color(parse_color(raw)?)));
    }
    if NUMERIC_KEYS.contains(&root) {
        let size = parse_font_size(raw)?;
        return canonical_number(size.points())
            .map(MarkValue)
            .ok_or_else(|| MarkValueError::BadFontSize(raw.to_owned()));
    }
    Ok(MarkValue(raw.to_owned()))
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
pub fn canonical_value(mark_key: &MarkKey, raw: &str) -> Option<MarkValue> {
    if !MarkRoot::of(mark_key).is_some_and(|root| NUMERIC_KEYS.contains(&root)) {
        return Some(MarkValue(raw.to_owned()));
    }
    canonical_number(raw.parse::<f32>().ok()?).map(MarkValue)
}
