//! The request types of the two spelling modules
//! (`4-Prose/features/ambre-spec-prose-spell-check.md`).
//!
//! A spelling module holds the spell checker and one dictionary. A check
//! hands it the words of some paragraphs and gets back those the dictionary
//! does not know. A suggestion hands it one word. The module never sees a
//! paragraph, a position or a document: the application keeps where each
//! word sits.

use ambre_core::index::Axis;
use ambre_core::{ConverterFamily, Count, NarrowCount, Rule, Validated};
use serde::{Deserialize, Serialize};

/// The dictionaries the suite ships, one spelling module each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dictionary {
    /// American English, from SCOWL.
    English,
    /// French, from Grammalecte.
    French,
}

impl Dictionary {
    /// The family whose module holds this dictionary.
    #[must_use]
    pub const fn family(self) -> ConverterFamily {
        match self {
            Self::English => ConverterFamily::SpellEn,
            Self::French => ConverterFamily::SpellFr,
        }
    }

    /// The dictionary of the language `tag` names, by its primary subtag:
    /// `en-GB` reads English, `fr-CA` French. Any other language has none.
    #[must_use]
    pub fn of_language(tag: &Validated<ambre_core::lang::LangTag>) -> Option<Self> {
        match ambre_core::lang::primary_subtag(tag).as_str() {
            "en" => Some(Self::English),
            "fr" => Some(Self::French),
            _ => None,
        }
    }
}

/// The rule of a word a module checks or suggests: a text of one to
/// [`MAX_WORD_CHARS`] code points with no control character. A suggestion
/// may hold a space or a hyphen, as `he lo` does, so the rule admits both.
pub enum SpellWord {}

/// The code points of a word.
pub enum WordChars {}

impl NarrowCount for WordChars {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("WordCharCount");
}

/// The longest word the checker reads, in code points. A longer run of
/// letters is no word of any dictionary, and a check skips it.
pub const MAX_WORD_CHARS: Count<WordChars> = Count::new(64);

/// A text that is no word: empty, too long, or holding a control character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a word holds one to 64 characters and no control character")]
pub struct NotAWord;

impl Rule for SpellWord {
    type Error = NotAWord;

    fn admit(text: String) -> Result<String, NotAWord> {
        let chars = Count::<WordChars>::of(text.chars().count());
        if !chars.is_zero() && chars <= MAX_WORD_CHARS && !text.chars().any(char::is_control) {
            Ok(text)
        } else {
            Err(NotAWord)
        }
    }
}

/// The position of a word in the list a check hands the module.
#[derive(Debug, PartialEq, Eq)]
pub enum CheckedWord {}

impl Axis for CheckedWord {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CheckedWord");
}

/// The suggestions a module answers for one word.
pub enum Suggested {}

impl NarrowCount for Suggested {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SuggestionCount");
}

/// The most suggestions a module answers for one word.
pub const MAX_SUGGESTIONS: Count<Suggested> = Count::new(5);

/// A check: the words to look up, under one dictionary. Answered by
/// [`crate::Converted::Misspelled`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpellCheck {
    pub dictionary: Dictionary,
    pub words: Vec<Validated<SpellWord>>,
}

/// A suggestion: the word to correct, under one dictionary. Answered by
/// [`crate::Converted::Suggestions`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpellSuggest {
    pub dictionary: Dictionary,
    pub word: Validated<SpellWord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_short_and_holds_no_control_character() {
        assert!(Validated::<SpellWord>::parse("maison").is_ok());
        assert!(Validated::<SpellWord>::parse("he lo").is_ok());
        assert!(Validated::<SpellWord>::parse("").is_err());
        assert!(Validated::<SpellWord>::parse("a\tb").is_err());
        assert!(Validated::<SpellWord>::parse("x".repeat(64)).is_ok());
        assert!(Validated::<SpellWord>::parse("x".repeat(65)).is_err());
    }

    #[test]
    fn each_dictionary_has_its_own_module() {
        assert_eq!(Dictionary::English.family(), ConverterFamily::SpellEn);
        assert_eq!(Dictionary::French.family(), ConverterFamily::SpellFr);
        let tag = |text: &str| Validated::<ambre_core::lang::LangTag>::parse(text).unwrap();
        assert_eq!(
            Dictionary::of_language(&tag("fr-CA")),
            Some(Dictionary::French)
        );
        assert_eq!(
            Dictionary::of_language(&tag("EN")),
            Some(Dictionary::English)
        );
        assert_eq!(Dictionary::of_language(&tag("de")), None);
    }
}
