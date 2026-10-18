//! A search over the segments a channel holds
//! (`specs/5-Murmure/features/ambre-spec-murmure-search.md`).
//!
//! It reads memory alone: no query leaves the client, and no segment is
//! loaded for it. A word matches when it begins a word of the message, both
//! cut by UAX #29 and folded to lowercase without diacritics.

use unicode_segmentation::UnicodeSegmentation;

/// The words of a query, folded. A query holds at least one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    words: Vec<String>,
}

impl Query {
    /// The words of `text`, or `None` when it holds no word.
    pub fn parse(text: &str) -> Option<Self> {
        let words = folded_words(text);
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

/// `word` lowercased, with the diacritics of the Latin letters dropped.
///
/// The fold covers Latin-1 and Latin Extended-A, and drops any combining
/// mark, so a decomposed accent folds too.
fn fold(word: &str) -> String {
    let mut folded = String::with_capacity(word.len());
    for c in word.chars().flat_map(char::to_lowercase) {
        match latin_base(c) {
            Some(base) => folded.push_str(base),
            None if ('\u{0300}'..='\u{036F}').contains(&c) => {}
            None => folded.push(c),
        }
    }
    folded
}

/// The base letters of a lowercase Latin letter that carries a diacritic.
fn latin_base(c: char) -> Option<&'static str> {
    Some(match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' | 'ĸ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' | 'ŋ' => "n",
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ſ' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' => "t",
        'þ' => "th",
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words fold to lowercase without their diacritics, composed or
    /// decomposed, so either spelling finds the other.
    #[test]
    fn a_query_matches_its_words_with_diacritics_folded() {
        let query = Query::parse("Éte bud").expect("two words");
        assert!(query.matches("the budget for the été"));
        assert!(query.matches("ETE: BUDGET"));
        assert!(
            query.matches("e\u{0301}te\u{0301} budget"),
            "a decomposed accent"
        );
        assert!(!query.matches("the budget alone"), "every word is needed");
        assert!(!query.matches("debud ete"), "a word matches from its start");
        assert_eq!(fold("Œuvre Straße Łódź"), "oeuvre strasse lodz");
        assert_eq!(Query::parse(" ,;! "), None, "no word, no query");
    }
}
