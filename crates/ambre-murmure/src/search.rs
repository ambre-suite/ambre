//! A search over the segments a channel holds
//! (`specs/5-Murmure/features/ambre-spec-murmure-search.md`).
//!
//! It reads memory alone: no query leaves the client, and no segment is
//! loaded for it. A word matches when it begins a word of the message, both
//! cut by UAX #29 and folded to lowercase without diacritics.

use unicode_segmentation::UnicodeSegmentation;

/// The words of a query, folded. A query holds at least one.
///
/// `Debug` counts the words alone: a member typed them.
#[derive(Clone, PartialEq, Eq)]
pub struct Query {
    words: Vec<String>,
}

impl std::fmt::Debug for Query {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Query")
            .field("words", &self.words.len())
            .finish()
    }
}

impl Query {
    /// The words of `query`, or `None` when it holds no word.
    pub fn parse(query: &ambre_core::Validated<ambre_core::SearchQuery>) -> Option<Self> {
        // The text leaves the newtype at the foreign word cutter, not at a
        // workspace helper, so the unwrap report has nothing to refuse.
        let words: Vec<String> = query.as_str().unicode_words().map(fold).collect();
        (!words.is_empty()).then_some(Self { words })
    }

    /// Whether every word of the query begins a word of `text`.
    pub(crate) fn matches(&self, text: &str) -> bool {
        let words = folded_words(text);
        self.words
            .iter()
            .all(|wanted| words.iter().any(|word| word.starts_with(wanted.as_str())))
    }
}

/// The words of `text`, each lowercased and folded.
fn folded_words(text: &str) -> Vec<String> {
    text.unicode_words().map(fold).collect()
}

/// `word` lowercased, with its diacritics dropped.
///
/// A precomposed Latin, Greek or Cyrillic letter folds to its base by the
/// table of [`base`]. A combining mark of any script is dropped, so a
/// decomposed accent folds too.
fn fold(word: &str) -> String {
    let mut folded = String::with_capacity(word.len());
    for c in word.chars().flat_map(char::to_lowercase) {
        match base(c) {
            Some(base) => folded.push_str(base),
            None if is_mark(c) => {}
            None => folded.push(c),
        }
    }
    folded
}

/// The base letters of a lowercase letter that carries a diacritic.
///
/// The ranges hold the canonical decompositions of Unicode, from Latin-1 to
/// Latin Extended Additional, Greek with Greek Extended, and Cyrillic. An
/// uppercase letter inside a range never reaches it, being lowercased first.
/// A Cyrillic letter of its own, such as `й`, stays as typed.
fn base(c: char) -> Option<&'static str> {
    Some(match c {
        'à'..='å'
        | 'ā'..='ą'
        | 'ǎ'
        | 'ǟ'..='ǡ'
        | 'ǻ'
        | 'ȁ'..='ȃ'
        | 'ȧ'
        | 'ḁ'
        | '\u{1EA1}'..='\u{1EB7}' => "a",
        'æ' | 'ǣ' | 'ǽ' => "ae",
        'ḃ'..='ḇ' => "b",
        'ç' | 'ć'..='č' | 'ḉ' => "c",
        'ď' | 'đ' | 'ð' | 'ḋ'..='ḓ' => "d",
        'è'..='ë' | 'ē'..='ě' | 'ȅ'..='ȇ' | 'ȩ' | 'ḕ'..='ḝ' | '\u{1EB9}'..='\u{1EC7}' => {
            "e"
        }
        'ḟ' => "f",
        'ĝ'..='ģ' | 'ǧ' | 'ǵ' | 'ḡ' => "g",
        'ĥ' | 'ħ' | 'ȟ' | 'ḣ'..='ḫ' | 'ẖ' => "h",
        'ì'..='ï' | 'ĩ'..='į' | 'ı' | 'ǐ' | 'ȉ'..='ȋ' | 'ḭ'..='ḯ' | 'ỉ'..='ị' => {
            "i"
        }
        'ĳ' => "ij",
        'ĵ' | 'ǰ' => "j",
        'ķ' | 'ĸ' | 'ǩ' | 'ḱ'..='ḵ' => "k",
        'ĺ'..='ł' | 'ḷ'..='ḽ' => "l",
        'ḿ'..='ṃ' => "m",
        'ñ' | 'ń'..='ŋ' | 'ǹ' | 'ṅ'..='ṋ' => "n",
        'ò'..='ö'
        | 'ø'
        | 'ō'..='ő'
        | 'ơ'
        | 'ǒ'
        | 'ǫ'..='ǭ'
        | 'ǿ'
        | 'ȍ'..='ȏ'
        | 'ȫ'..='ȱ'
        | 'ṍ'..='ṓ'
        | '\u{1ECD}'..='\u{1EE3}' => "o",
        'œ' => "oe",
        'ṕ'..='ṗ' => "p",
        'ŕ'..='ř' | 'ȑ'..='ȓ' | 'ṙ'..='ṟ' => "r",
        'ś'..='š' | 'ſ' | 'ș' | 'ṡ'..='ṩ' | 'ẛ' => "s",
        'ß' => "ss",
        'ţ'..='ŧ' | 'ț' | 'ṫ'..='ṱ' | 'ẗ' => "t",
        'þ' => "th",
        'ù'..='ü'
        | 'ũ'..='ų'
        | 'ư'
        | 'ǔ'..='ǜ'
        | 'ȕ'..='ȗ'
        | 'ṳ'..='ṻ'
        | '\u{1EE5}'..='\u{1EF1}' => "u",
        'ṽ'..='ṿ' => "v",
        'ŵ' | 'ẁ'..='ẉ' | 'ẘ' => "w",
        'ẋ'..='ẍ' => "x",
        'ý' | 'ÿ' | 'ŷ' | 'ȳ' | 'ẏ' | 'ẙ' | '\u{1EF3}'..='\u{1EF9}' => "y",
        'ź'..='ž' | 'ẑ'..='ẕ' => "z",
        // Greek, with its tonos (U+03AC to U+03CE) and its polytonic forms
        // (U+1F00 to U+1FF7), spelled by code point: the two acute accents
        // look alike.
        '\u{03AC}'
        | '\u{1F00}'..='\u{1F07}'
        | '\u{1F70}'..='\u{1F71}'
        | '\u{1F80}'..='\u{1F87}'
        | '\u{1FB0}'..='\u{1FB7}' => "α",
        '\u{03AD}' | '\u{1F10}'..='\u{1F15}' | '\u{1F72}'..='\u{1F73}' => "ε",
        '\u{03AE}'
        | '\u{1F20}'..='\u{1F27}'
        | '\u{1F74}'..='\u{1F75}'
        | '\u{1F90}'..='\u{1F97}'
        | '\u{1FC2}'..='\u{1FC7}' => "η",
        '\u{0390}'
        | '\u{03AF}'
        | '\u{03CA}'
        | '\u{1F30}'..='\u{1F37}'
        | '\u{1F76}'..='\u{1F77}'
        | '\u{1FBE}'
        | '\u{1FD0}'..='\u{1FD3}'
        | '\u{1FD6}'..='\u{1FD7}' => "ι",
        '\u{03CC}' | '\u{1F40}'..='\u{1F45}' | '\u{1F78}'..='\u{1F79}' => "ο",
        '\u{1FE4}'..='\u{1FE5}' => "ρ",
        'ς' => "σ",
        '\u{03B0}'
        | '\u{03CB}'
        | '\u{03CD}'
        | '\u{1F50}'..='\u{1F57}'
        | '\u{1F7A}'..='\u{1F7B}'
        | '\u{1FE0}'..='\u{1FE3}'
        | '\u{1FE6}'..='\u{1FE7}' => "υ",
        '\u{03CE}'
        | '\u{1F60}'..='\u{1F67}'
        | '\u{1F7C}'..='\u{1F7D}'
        | '\u{1FA0}'..='\u{1FA7}'
        | '\u{1FF2}'..='\u{1FF7}' => "ω",
        'ѐ' | 'ё' => "е",
        'ѝ' => "и",
        _ => return None,
    })
}

/// Whether `c` is a combining mark: an accent, a Hebrew point or an Arabic
/// vowel sign, written after the letter it marks.
fn is_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{0483}'..='\u{0489}'
            | '\u{0591}'..='\u{05BD}'
            | '\u{05BF}'
            | '\u{05C1}'..='\u{05C2}'
            | '\u{05C4}'..='\u{05C5}'
            | '\u{05C7}'
            | '\u{0610}'..='\u{061A}'
            | '\u{064B}'..='\u{065F}'
            | '\u{0670}'
            | '\u{06D6}'..='\u{06DC}'
            | '\u{06DF}'..='\u{06E4}'
            | '\u{06E7}'..='\u{06E8}'
            | '\u{06EA}'..='\u{06ED}'
            | '\u{1AB0}'..='\u{1ACE}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20F0}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The query `text` parses to, typed as a member types a search.
    fn parse(text: &str) -> Option<Query> {
        Query::parse(
            &ambre_core::Validated::<ambre_core::SearchQuery>::parse(text.to_owned())
                .expect("not blank"),
        )
    }

    /// The debug form of a query counts its words, never a char of them.
    #[test]
    fn a_query_debugs_its_word_count_alone() {
        let query = parse("secret budget").expect("two words");
        let shown = format!("{query:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("words: 2"), "{shown}");
    }

    /// The words fold to lowercase without their diacritics, composed or
    /// decomposed, so either spelling finds the other.
    #[test]
    fn a_query_matches_its_words_with_diacritics_folded() {
        let query = parse("Éte bud").expect("two words");
        assert!(query.matches("the budget for the été"));
        assert!(query.matches("ETE: BUDGET"));
        assert!(
            query.matches("e\u{0301}te\u{0301} budget"),
            "a decomposed accent"
        );
        assert!(!query.matches("the budget alone"), "every word is needed");
        assert!(!query.matches("debud ete"), "a word matches from its start");
        assert_eq!(fold("Œuvre Straße Łódź"), "oeuvre strasse lodz");
        assert_eq!(parse(" ,;! "), None, "no word, no query");
    }

    /// Past Latin-1, each script folds its precomposed letters to their base
    /// and drops its combining marks. A Cyrillic letter of its own stays.
    #[test]
    fn every_script_folds_its_diacritics() {
        for (typed, folded) in [
            ("Tiếng Việt", "tieng viet"),
            ("nǚ lǜ", "nu lu"),
            ("Ἀθῆναι ὁδός", "αθηναι οδοσ"),
            ("Ελλάδα", "ελλαδα"),
            ("ёлка", "елка"),
            // Hebrew points and Arabic vowel signs, spelled by code point.
            ("ש\u{05C1}\u{05B8}לו\u{05B9}ם", "שלום"),
            ("ك\u{064E}ت\u{064E}ب\u{064E}", "كتب"),
            ("Ș ț", "s t"),
        ] {
            assert_eq!(fold(typed), folded, "{typed}");
        }
        assert_eq!(fold("йод"), "йод", "й is a letter, not an и");
        let query = parse("Ελλαδα").expect("a word");
        assert!(query.matches("η Ελλάδα"));
    }
}
