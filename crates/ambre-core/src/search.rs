//! The texts a search carries: a message search, which reads only what the
//! client holds, and a user search, which the relay answers.

use crate::Redacted;
use serde::{Deserialize, Serialize};

/// The text of a message search, as typed, holding at least one character
/// that is not white space. The box sends no search for a blank text.
/// `Debug` prints the length alone, so a log carries none of the text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SearchQuery(Redacted<String>);

impl SearchQuery {
    /// The search for `text`, kept as typed; `None` when it is blank.
    #[must_use]
    pub fn new(text: String) -> Option<Self> {
        (!text.trim().is_empty()).then_some(Self(Redacted::new(text)))
    }

    /// The text as typed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<String> for SearchQuery {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text).ok_or("a search is never blank")
    }
}

impl From<SearchQuery> for String {
    fn from(query: SearchQuery) -> Self {
        query.0.into_inner()
    }
}

/// The text a user search matches, trimmed and at least two characters long.
/// A shorter text would match a near-arbitrary slice of the user table. The
/// sharing lookup would then list the directory, each email and key with it.
/// `Debug` prints the length alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UserSearch(Redacted<String>);

impl UserSearch {
    /// The search for `text`, `None` when its trimmed text is shorter than two
    /// characters.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        (text.chars().count() >= 2).then(|| Self(Redacted::new(text.to_owned())))
    }

    /// The trimmed text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<String> for UserSearch {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text).ok_or("a user search holds at least two characters")
    }
}

impl From<UserSearch> for String {
    fn from(search: UserSearch) -> Self {
        search.0.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::{SearchQuery, UserSearch};

    /// Both searches print their length in `Debug`, and none of the text.
    #[test]
    fn a_search_prints_its_length_alone() {
        let query = SearchQuery::new("salary review".to_owned()).expect("a query");
        assert_eq!(format!("{query:?}"), "SearchQuery(13 chars)");
        let search = UserSearch::new(" ada ").expect("a search");
        assert_eq!(format!("{search:?}"), "UserSearch(3 chars)");
    }

    /// A message search keeps its text as typed, and refuses a blank one,
    /// from the box as from the bytes.
    #[test]
    fn a_message_search_is_never_blank() {
        let query = SearchQuery::new(" bud ".to_owned()).expect("a word");
        assert_eq!(query.as_str(), " bud ");
        for blank in ["", " ", "\t\n"] {
            assert_eq!(SearchQuery::new(blank.to_owned()), None, "{blank:?}");
            let bytes = postcard::to_allocvec(blank).expect("a string encodes");
            assert!(
                postcard::from_bytes::<SearchQuery>(&bytes).is_err(),
                "{blank:?}"
            );
        }
    }

    /// A search keeps its trimmed text, and refuses one shorter than two
    /// characters, counted as characters rather than bytes.
    #[test]
    fn a_search_is_two_characters_or_none() {
        assert_eq!(
            UserSearch::new("  al ").map(|s| s.0.into_inner()),
            Some("al".to_owned())
        );
        for short in ["", "   ", "a", " é "] {
            assert_eq!(UserSearch::new(short), None, "{short:?}");
        }
        assert!(UserSearch::new("éé").is_some());
    }

    /// A user search travels as its bare text, and the bytes of a text too
    /// short to search are refused.
    #[test]
    fn a_user_search_travels_as_its_text_and_refuses_a_short_one() {
        let search = UserSearch::new(" ab ").expect("two characters");
        let bytes = postcard::to_allocvec(&search).expect("a search encodes");
        assert_eq!(
            bytes,
            postcard::to_allocvec("ab").expect("a string encodes")
        );
        assert_eq!(postcard::from_bytes::<UserSearch>(&bytes), Ok(search));
        for short in ["", "a", " a "] {
            let bytes = postcard::to_allocvec(short).expect("a string encodes");
            assert!(
                postcard::from_bytes::<UserSearch>(&bytes).is_err(),
                "{short:?}"
            );
        }
    }
}
