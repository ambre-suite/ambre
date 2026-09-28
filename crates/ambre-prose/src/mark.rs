//! The inline mark registry: what a mark key means, how its value is spelled,
//! and what an adjacent insertion does to it
//! (`ambre-spec-prose-schema.md section 3`).
//!
//! [`expand_type_for`] is the one place a mark's expansion behaviour is decided
//! (`PROSE-007`). [`configure`] builds the document's whole style configuration
//! from it. A call site cannot pick a different policy for the same key,
//! because it never states one.

use loro::{ExpandType, LoroDoc, StyleConfig, StyleConfigMap};

/// The formatting keys: the mark's value is the formatting itself, and an
/// insertion at the edge of the range takes the formatting with it.
///
/// `ClearInlineMarks` removes exactly these. A comment, a suggestion, a
/// footnote and a cross reference are anchors rather than formatting, and each
/// has a feature that owns its removal.
pub const FORMATTING_KEYS: [&str; 12] = [
    "b", "i", "u", "s", "sup", "sub", "c", "a", "fg", "bg", "fs", "ff",
];

/// The keys whose value is an identifier or a record, and which therefore must
/// never swallow an adjacent insertion.
///
/// Loro reads a key of the shape `a:b` under its root `a`. The one entry `f`
/// therefore covers `f:pn`, `f:pc`, `f:dt` and `f:ti`.
const ANCHOR_KEYS: [&str; 6] = ["a", "cm", "si", "sd", "fn", "ref"];

/// The header-field root, registered alone for the reason above.
const FIELD_ROOT: &str = "f";

/// The roots a command of their own owns: a comment, a suggestion, a
/// footnote, a cross reference and a header field. The link is an anchor too,
/// and the one the style path writes.
const OWNED_ROOTS: [&str; 6] = ["cm", "si", "sd", "fn", "ref", FIELD_ROOT];

/// Whether a key belongs to an anchor its own command writes and removes, so
/// the style path must refuse it (`PROSE-FEAT-298`, `PROSE-FEAT-299`).
#[must_use]
pub fn is_owned_anchor(mark_key: &str) -> bool {
    let root = mark_key.split(':').next().unwrap_or(mark_key);
    OWNED_ROOTS.contains(&root)
}

/// The expansion an insertion at the edge of a mark gets.
///
/// Total over every key (`PROSE-090`): a key no build knows answers the
/// default, which is what a formatting mark does. A panic here would kill the
/// worker over a document written by a newer client.
#[must_use]
pub fn expand_type_for(mark_key: &str) -> ExpandType {
    // A key of the shape `a:b` belongs to the registry entry `a`, which is how
    // Loro itself resolves one.
    let root = mark_key.split(':').next().unwrap_or(mark_key);
    if root == FIELD_ROOT || ANCHOR_KEYS.contains(&root) {
        ExpandType::None
    } else {
        ExpandType::After
    }
}

/// Every root the registry declares, which is every key a document may carry a
/// mark under.
///
/// Loro refuses a mark whose key it holds no configuration for. This list is
/// therefore what a command may write a mark under.
fn registered_roots() -> impl Iterator<Item = &'static str> {
    FORMATTING_KEYS
        .into_iter()
        .chain(ANCHOR_KEYS)
        .chain([FIELD_ROOT])
}

/// Whether a command may write a mark under this key.
///
/// The CRDT engine holds a configuration per registered root and refuses every
/// other key. A command that names one is therefore refused rather than
/// silently dropped. The check sits here, so the refusal names the registry
/// rather than the engine.
#[must_use]
pub fn is_registered(mark_key: &str) -> bool {
    let root = mark_key.split(':').next().unwrap_or(mark_key);
    registered_roots().any(|known| known == root)
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
    for key in registered_roots() {
        styles.insert(
            key.into(),
            StyleConfig {
                expand: expand_type_for(key),
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
const FLAG_KEYS: [&str; 7] = ["b", "i", "u", "s", "sup", "sub", "c"];

/// The keys whose value is a colour.
const COLOR_KEYS: [&str; 2] = ["fg", "bg"];

/// The largest font size the office formats round-trip, in points.
const FONT_SIZE_MAX: f32 = 1638.0;

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
    ArityMismatch { key: String },
    /// A key the registry does not declare.
    UnknownKey(String),
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
pub fn parse_color(raw: &str) -> Result<u32, MarkValueError> {
    let bad = || MarkValueError::BadColor(raw.to_owned());
    let digits = raw.strip_prefix('#').ok_or_else(bad)?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16)
            .map(|rgb| (rgb << 8) | 0xFF)
            .map_err(|_| bad()),
        8 => u32::from_str_radix(digits, 16).map_err(|_| bad()),
        _ => Err(bad()),
    }
}

/// The one spelling a parsed colour is stored as (`PROSE-FEAT-175`).
///
/// A fully opaque colour is spelled in six digits, and every other in eight.
/// Lower case throughout, so two peers writing one colour write one string.
#[must_use]
pub fn canonical_color(rgba: u32) -> String {
    if rgba & 0xFF == 0xFF {
        format!("#{:06x}", rgba >> 8)
    } else {
        format!("#{rgba:08x}")
    }
}

/// Parse a size in points. The ceiling is the one the office formats
/// round-trip.
///
/// # Errors
/// [`MarkValueError::BadFontSize`] for a value that does not read as a number,
/// is not finite, or falls outside one to 1638 points.
pub fn parse_font_size(raw: &str) -> Result<f32, MarkValueError> {
    let bad = || MarkValueError::BadFontSize(raw.to_owned());
    let points = raw.parse::<f32>().map_err(|_| bad())?;
    if !points.is_finite() || !(1.0..=FONT_SIZE_MAX).contains(&points) {
        return Err(bad());
    }
    Ok(points)
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
pub fn validate_value(key: &str, raw: Option<&str>) -> Result<String, MarkValueError> {
    let root = key.split(':').next().unwrap_or(key);
    if !is_registered(key) {
        return Err(MarkValueError::UnknownKey(key.to_owned()));
    }
    let arity = || MarkValueError::ArityMismatch {
        key: key.to_owned(),
    };
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
        let points = parse_font_size(raw)?;
        return canonical_number(points).ok_or_else(|| MarkValueError::BadFontSize(raw.to_owned()));
    }
    Ok(raw.to_owned())
}

/// The keys whose value is a number, and the only ones a re-spelling applies to.
///
/// Every other key of section 3 carries text: a URL, a family name, a `Uuid`,
/// a hex record, a `#rrggbb` colour. The float parser would rewrite those. A
/// cross reference of `"1e5"` reads as a number, and a footnote of `"007"`
/// loses its zeroes. So the grammar is read from the key.
const NUMERIC_KEYS: [&str; 1] = ["fs"];

/// The canonical spelling of a mark value a command carried, or `None` when the
/// value has none (`PROSE-008`, `PROSE-091`).
///
/// A value under a numeric key is re-spelled through [`canonical_number`], so
/// `"11.0"` and `"11"` reach the document as one string. A value that is not a
/// finite number is refused. Every other key's value is its own canonical form:
/// a URL, a family name and a `Uuid` are spelled once.
#[must_use]
pub fn canonical_value(mark_key: &str, raw: &str) -> Option<String> {
    let root = mark_key.split(':').next().unwrap_or(mark_key);
    if !NUMERIC_KEYS.contains(&root) {
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
            assert_eq!(expand_type_for(key), ExpandType::After, "{key}");
        }
    }

    #[test]
    fn an_anchor_mark_never_swallows_an_insertion() {
        for key in ANCHOR_KEYS {
            assert_eq!(expand_type_for(key), ExpandType::None, "{key}");
        }
    }

    #[test]
    fn a_header_field_is_read_under_its_own_root() {
        for key in ["f:pn", "f:pc", "f:dt", "f:ti"] {
            assert_eq!(expand_type_for(key), ExpandType::None, "{key}");
            assert!(is_registered(key), "{key}");
        }
    }

    #[test]
    fn an_unknown_key_answers_the_default_and_never_panics() {
        assert_eq!(expand_type_for("a-key-no-build-knows"), ExpandType::After);
        assert_eq!(expand_type_for(""), ExpandType::After);
        assert_eq!(expand_type_for(":"), ExpandType::After);
        assert!(!is_registered("a-key-no-build-knows"));
    }

    #[test]
    fn two_peers_spell_one_size_the_same_way() {
        assert_eq!(canonical_value("fs", "11.0").as_deref(), Some("11"));
        assert_eq!(canonical_value("fs", "11").as_deref(), Some("11"));
        assert_eq!(canonical_value("fs", "11.50").as_deref(), Some("11.5"));
        assert_eq!(canonical_number(0.5).as_deref(), Some("0.5"));
        assert_eq!(canonical_number(f32::NAN), None);
        assert_eq!(canonical_number(f32::INFINITY), None);
    }

    #[test]
    fn a_value_that_is_not_a_number_is_its_own_canonical_form() {
        assert_eq!(
            canonical_value("a", "https://example.org/a").as_deref(),
            Some("https://example.org/a")
        );
        assert_eq!(canonical_value("fg", "#ff8800").as_deref(), Some("#ff8800"));
    }

    /// A value under a key the registry spells as text reaches the document as
    /// the user wrote it. Each of these reads as a number to the float parser,
    /// and re-spelling it would name another anchor or another font.
    #[test]
    fn a_textual_value_that_reads_as_a_number_is_left_alone() {
        assert_eq!(canonical_value("ref", "1.10").as_deref(), Some("1.10"));
        assert_eq!(canonical_value("fn", "007").as_deref(), Some("007"));
        assert_eq!(canonical_value("cm", "1e5").as_deref(), Some("1e5"));
        assert_eq!(canonical_value("ff", "inf").as_deref(), Some("inf"));
        assert_eq!(canonical_value("f:pn", "NaN").as_deref(), Some("NaN"));
    }

    /// The numeric key is the one that refuses. A size with no finite value
    /// has no canonical spelling. The command is refused, rather than the
    /// document left holding a marker no reader can use.
    #[test]
    fn a_size_that_is_not_a_finite_number_has_no_canonical_spelling() {
        assert_eq!(canonical_value("fs", "inf"), None);
        assert_eq!(canonical_value("fs", "eleven"), None);
    }
}
