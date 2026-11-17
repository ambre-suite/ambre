//! The spell checker of the two spelling modules
//! (`4-Prose/features/ambre-spec-prose-spell-check.md`).
//!
//! Each module embeds one Hunspell dictionary and parses it on its first
//! request. The parsed dictionary stays in the instance for the next ones, so
//! a check after the first costs a lookup per word. The converter thread
//! runs these requests, never the Logic thread.

use std::cell::OnceCell;

use ambre_core::index::Pos;
use ambre_core::{FileRefusal, Validated};

use crate::spelling::{
    CheckedWord, Dictionary, MAX_SUGGESTIONS, SpellCheck, SpellSuggest, SpellWord,
};

thread_local! {
    /// The parsed English dictionary, once the first request asked for it.
    #[cfg(feature = "spell_en")]
    static ENGLISH: OnceCell<Option<spellbook::Dictionary>> = const { OnceCell::new() };
    /// The parsed French dictionary, once the first request asked for it.
    #[cfg(feature = "spell_fr")]
    static FRENCH: OnceCell<Option<spellbook::Dictionary>> = const { OnceCell::new() };
}

/// Run `read` over the parsed `dictionary`.
///
/// # Errors
///
/// [`FileRefusal::NotCarried`] when this build embeds no such dictionary, or
/// when the embedded one does not parse.
fn with<T>(
    dictionary: Dictionary,
    read: impl FnOnce(&spellbook::Dictionary) -> T,
) -> Result<T, FileRefusal> {
    let parsed = |cell: &OnceCell<Option<spellbook::Dictionary>>, aff: &str, dic: &str| {
        cell.get_or_init(|| spellbook::Dictionary::new(aff, dic).ok())
            .as_ref()
            .map(read)
            .ok_or(FileRefusal::NotCarried)
    };
    match dictionary {
        #[cfg(feature = "spell_en")]
        Dictionary::English => ENGLISH.with(|cell| {
            parsed(
                cell,
                include_str!("../../dictionaries/en_US.aff"),
                include_str!("../../dictionaries/en_US.dic"),
            )
        }),
        #[cfg(feature = "spell_fr")]
        Dictionary::French => FRENCH.with(|cell| {
            parsed(
                cell,
                include_str!("../../dictionaries/fr.aff"),
                include_str!("../../dictionaries/fr.dic"),
            )
        }),
        #[cfg(not(all(feature = "spell_en", feature = "spell_fr")))]
        _ => Err(FileRefusal::NotCarried),
    }
}

/// The positions of the words of `check` its dictionary does not know.
///
/// # Errors
///
/// [`FileRefusal::NotCarried`] when this build embeds no such dictionary.
pub(super) fn check(check: &SpellCheck) -> Result<Vec<Pos<CheckedWord>>, FileRefusal> {
    with(check.dictionary, |dictionary| {
        check
            .words
            .iter()
            .enumerate()
            // A typographic apostrophe reads as the straight one, the form
            // both dictionaries list.
            .filter(|(_, word)| !dictionary.check(&word.as_str().replace('\u{2019}', "'")))
            .map(|(at, _)| Pos::<CheckedWord>::from_usize(at))
            .collect()
    })
}

/// At most [`MAX_SUGGESTIONS`] corrections of the word of `suggest`, best
/// first.
///
/// # Errors
///
/// [`FileRefusal::NotCarried`] when this build embeds no such dictionary.
pub(super) fn suggest(suggest: &SpellSuggest) -> Result<Vec<Validated<SpellWord>>, FileRefusal> {
    with(suggest.dictionary, |dictionary| {
        let mut found = Vec::new();
        dictionary.suggest(&suggest.word.as_str().replace('\u{2019}', "'"), &mut found);
        let mut kept = Vec::new();
        for word in found
            .into_iter()
            .filter_map(|w| Validated::<SpellWord>::parse(w).ok())
        {
            if ambre_core::Count::of(kept.len()) >= MAX_SUGGESTIONS {
                break;
            }
            kept.push(word);
        }
        kept
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spelling::Dictionary;

    fn words(texts: &[&str]) -> Vec<Validated<SpellWord>> {
        texts
            .iter()
            .map(|text| Validated::<SpellWord>::parse(*text).expect("a word"))
            .collect()
    }

    fn misspelled(dictionary: Dictionary, texts: &[&str]) -> Vec<usize> {
        check(&SpellCheck {
            dictionary,
            words: words(texts),
        })
        .expect("the host embeds both dictionaries")
        .into_iter()
        .map(Pos::to_usize)
        .collect()
    }

    /// An English check names the words the dictionary
    /// does not know, an apostrophe of either form read as one.
    #[test]
    fn an_english_check_names_the_unknown_words() {
        assert_eq!(
            misspelled(
                Dictionary::English,
                &["The", "house", "recieve", "don't", "don\u{2019}t", "teh"]
            ),
            vec![2, 5]
        );
    }

    /// A French check names the words the dictionary does
    /// not know, an elision included.
    #[test]
    fn a_french_check_names_the_unknown_words() {
        assert_eq!(
            misspelled(
                Dictionary::French,
                &[
                    "maison",
                    "maisn",
                    "l'arbre",
                    "l\u{2019}arbre",
                    "\u{e9}taient"
                ]
            ),
            vec![1]
        );
    }

    /// A suggestion answers at most five corrections, the
    /// right one among them.
    #[test]
    fn a_suggestion_answers_at_most_five_corrections() {
        for (dictionary, word, right) in [
            (Dictionary::English, "recieve", "receive"),
            (Dictionary::English, "helo", "hello"),
            (Dictionary::French, "maisn", "maison"),
        ] {
            let found = suggest(&SpellSuggest {
                dictionary,
                word: Validated::<SpellWord>::parse(word).expect("a word"),
            })
            .expect("the host embeds both dictionaries");
            assert!(
                ambre_core::Count::of(found.len()) <= MAX_SUGGESTIONS,
                "{word}: {}",
                found.len()
            );
            assert!(
                found.iter().any(|w| w.as_str() == right),
                "{word} offers {right}"
            );
        }
    }
}
