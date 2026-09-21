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

/// The canonical spelling of a mark value a command carried, or `None` when the
/// value has none (`PROSE-008`, `PROSE-091`).
///
/// A value that reads as a number is re-spelled through [`canonical_number`], so
/// `"11.0"` and `"11"` reach the document as one string. Every other value is
/// its own canonical form: a URL, a family name and a `Uuid` are spelled once.
#[must_use]
pub fn canonical_value(raw: &str) -> Option<String> {
    match raw.parse::<f32>() {
        Ok(number) => canonical_number(number),
        Err(_) => Some(raw.to_owned()),
    }
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
        assert_eq!(canonical_value("11.0").as_deref(), Some("11"));
        assert_eq!(canonical_value("11").as_deref(), Some("11"));
        assert_eq!(canonical_value("11.50").as_deref(), Some("11.5"));
        assert_eq!(canonical_number(0.5).as_deref(), Some("0.5"));
        assert_eq!(canonical_number(f32::NAN), None);
        assert_eq!(canonical_number(f32::INFINITY), None);
    }

    #[test]
    fn a_value_that_is_not_a_number_is_its_own_canonical_form() {
        assert_eq!(
            canonical_value("https://example.org/a").as_deref(),
            Some("https://example.org/a")
        );
        assert_eq!(canonical_value("#ff8800").as_deref(), Some("#ff8800"));
    }
}
