//! The inline mark registry: what a mark key means, how its value is spelled,
//! and what an adjacent insertion does to it
//! (`ambre-spec-prose-schema.md section 3`).
//!
//! [`expand_type_for`] is the one place a mark's expansion behaviour is decided
//! (`PROSE-007`). [`configure`] builds the document's whole style configuration
//! from it. A call site cannot pick a different policy for the same key,
//! because it never states one.

use ambre_bus::app::prose::MarkKey;
use loro::{ExpandType, LoroDoc, StyleConfig, StyleConfigMap};

// The value grammar lives on the bus, so a command carries a checked value.
pub use ambre_bus::app::prose::mark::{
    MarkRoot, MarkValue, MarkValueError, canonical_color, canonical_number, canonical_value,
    parse_color, parse_font_size, validate_value,
};

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

/// A value for a test command, checked as the text of a link, which takes any
/// text. The Logic thread checks the value again under the key the command
/// names, which is what the test exercises.
#[cfg(test)]
pub(crate) fn test_value(raw: &str) -> MarkValue {
    validate_value(&MarkRoot::Link.key(), Some(raw)).expect("a link takes any text")
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
        assert_eq!(
            canonical_value(&"fs".into(), "11.0")
                .as_ref()
                .map(MarkValue::as_str),
            Some("11")
        );
        assert_eq!(
            canonical_value(&"fs".into(), "11")
                .as_ref()
                .map(MarkValue::as_str),
            Some("11")
        );
        assert_eq!(
            canonical_value(&"fs".into(), "11.50")
                .as_ref()
                .map(MarkValue::as_str),
            Some("11.5")
        );
        assert_eq!(canonical_number(0.5).as_deref(), Some("0.5"));
        assert_eq!(canonical_number(f32::NAN), None);
        assert_eq!(canonical_number(f32::INFINITY), None);
    }

    #[test]
    fn a_value_that_is_not_a_number_is_its_own_canonical_form() {
        assert_eq!(
            canonical_value(&"a".into(), "https://example.org/a")
                .as_ref()
                .map(MarkValue::as_str),
            Some("https://example.org/a")
        );
        assert_eq!(
            canonical_value(&"fg".into(), "#ff8800")
                .as_ref()
                .map(MarkValue::as_str),
            Some("#ff8800")
        );
    }

    /// A value under a key the registry spells as text reaches the document as
    /// the user wrote it. Each of these reads as a number to the float parser,
    /// and re-spelling it would name another anchor or another font.
    #[test]
    fn a_textual_value_that_reads_as_a_number_is_left_alone() {
        assert_eq!(
            canonical_value(&"ref".into(), "1.10")
                .as_ref()
                .map(MarkValue::as_str),
            Some("1.10")
        );
        assert_eq!(
            canonical_value(&"fn".into(), "007")
                .as_ref()
                .map(MarkValue::as_str),
            Some("007")
        );
        assert_eq!(
            canonical_value(&"cm".into(), "1e5")
                .as_ref()
                .map(MarkValue::as_str),
            Some("1e5")
        );
        assert_eq!(
            canonical_value(&"ff".into(), "inf")
                .as_ref()
                .map(MarkValue::as_str),
            Some("inf")
        );
        assert_eq!(
            canonical_value(&"f:pn".into(), "NaN")
                .as_ref()
                .map(MarkValue::as_str),
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
