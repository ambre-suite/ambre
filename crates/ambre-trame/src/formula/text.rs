//! The three forms a cell's text takes (`TRAME-228`, `trame-sheets`). They
//! live in `ambre-bus`, because a deferred rule payload carries the source
//! form; this module keeps their engine path. It also holds the case fold
//! that every comparison ignoring case reads.

pub use ambre_bus::app::trame::{DisplayFormula, FormulaSource, StoredFormula};

/// One char lowercased, with a final sigma read as a plain one, as Unicode
/// case folding reads it. A whole-string lowercase picks the final sigma by
/// context, and a char alone cannot: folding char by char gives every reader
/// one form. A char may fold to several, as a capital I with a dot does.
pub(crate) fn fold_char(ch: char) -> impl Iterator<Item = char> {
    ch.to_lowercase()
        .map(|lower| if lower == '\u{3C2}' { '\u{3C3}' } else { lower })
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
