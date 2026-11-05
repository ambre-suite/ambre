//! The three forms a cell's text takes (`TRAME-228`, `trame-sheets`). They
//! live in `ambre-bus`, because a deferred rule payload carries the source
//! form; this module keeps their engine path. It also holds the case fold
//! that every comparison ignoring case reads.

pub use crate::message::{CellText, DisplayForm, SourceForm, StoredForm};

/// The text of a literal a user typed: a string in a formula, or the printed
/// part of a number format. It is document content, so its Debug form is its
/// length alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteralText(ambre_core::Redacted<String>);

impl LiteralText {
    /// The literal `text`.
    pub fn new(text: String) -> Self {
        Self(ambre_core::Redacted::new(text))
    }

    /// The text, for the value or the output that holds it.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// One char lowercased, with a final sigma read as a plain one, as Unicode
/// case folding reads it. A whole-string lowercase picks the final sigma by
/// context, and a char alone cannot: folding char by char gives every reader
/// one form. A char may fold to several, as a capital I with a dot does.
pub(crate) fn fold_char(ch: char) -> impl Iterator<Item = char> {
    ch.to_lowercase()
        .map(|lower| if lower == '\u{3C2}' { '\u{3C3}' } else { lower })
}

/// The order of two texts, their case ignored through [`fold_char`]: the one
/// comparison a lookup, a criterion and a find share.
pub(crate) fn fold_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.chars()
        .flat_map(fold_char)
        .cmp(b.chars().flat_map(fold_char))
}

/// One char folded to exactly one, for a reader that counts positions. A
/// char whose fold is several chars has no simple fold, so it stays itself.
/// Unicode simple case folding leaves a capital I with a dot so.
pub(crate) fn fold_char_simple(ch: char) -> char {
    let mut folded = fold_char(ch);
    match (folded.next(), folded.next()) {
        (Some(one), None) => one,
        _ => ch,
    }
}
