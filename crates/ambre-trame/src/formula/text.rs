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

/// The longest text a formula yields, in characters: the cell bound a
/// spreadsheet holds (`TRAME-313`, `ambre-spec-budgets.md section 6`). A
/// peer writes the formula, and each peer evaluates it on its Logic thread.
/// A text with no bound lets one signed delta allocate past the heap there.
pub(crate) const TEXT_MAX_CHARS: ambre_core::index::Len<ambre_core::Char> =
    ambre_core::index::Len::<ambre_core::Char>::new(32_767);

/// A text a formula would build past [`TEXT_MAX_CHARS`]. The cell shows
/// `#TYPE!`, as a spreadsheet shows `#VALUE!` there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextTooLong;

impl From<TextTooLong> for super::types::FormulaError {
    fn from(_: TextTooLong) -> Self {
        Self::TypeError
    }
}

/// A formula text under construction. It refuses a piece that would take it
/// past [`TEXT_MAX_CHARS`], before any allocation for that piece. So no
/// formula allocates past the bound, whatever its arguments (`TRAME-313`).
pub(crate) struct BoundedText {
    text: String,
    chars: ambre_core::index::Len<ambre_core::Char>,
}

impl BoundedText {
    /// An empty text.
    pub(crate) fn new() -> Self {
        Self {
            text: String::new(),
            chars: ambre_core::index::Len::ZERO,
        }
    }

    /// Append `piece`.
    ///
    /// # Errors
    ///
    /// [`TextTooLong`] when the text would pass the bound. Nothing is then
    /// appended.
    pub(crate) fn push(&mut self, piece: &str) -> Result<(), TextTooLong> {
        let chars = self.chars + ambre_core::char_len(piece);
        if chars > TEXT_MAX_CHARS {
            return Err(TextTooLong);
        }
        self.text.push_str(piece);
        self.chars = chars;
        Ok(())
    }

    /// The text as the value of a cell.
    pub(crate) fn into_value(self) -> super::types::CellValue {
        super::types::CellValue::Text(self.text)
    }
}
