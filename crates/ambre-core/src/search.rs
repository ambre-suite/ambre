//! The texts a search carries: a message search, which reads only what the
//! client holds, and a user search by address, which the relay answers.

use crate::Rule;

/// The rule of the text of a message search, as typed, holding at least one
/// character that is not white space. The box sends no search for a blank
/// text. A `Validated<SearchQuery>` holds one, and the rule keeps the text as
/// typed. `Debug` prints the length alone, so a log carries none of the text.
pub enum SearchQuery {}

impl Rule for SearchQuery {
    type Error = &'static str;

    fn admit(text: String) -> Result<String, Self::Error> {
        if text.trim().is_empty() {
            Err("a search is never blank")
        } else {
            Ok(text)
        }
    }
}

/// The rule of the text a user search carries: two characters at least once
/// trimmed. The relay answers only the account whose whole address equals the
/// text, in any case (`SRV-280`), so a fragment lists nobody. A
/// `Validated<UserSearch>` holds one, and the rule keeps the trimmed text.
/// `Debug` prints the length alone.
pub enum UserSearch {}

impl Rule for UserSearch {
    type Error = &'static str;

    fn admit(text: String) -> Result<String, Self::Error> {
        let trimmed = text.trim();
        if trimmed.chars().count() < 2 {
            Err("a user search holds at least two characters")
        } else if trimmed.len() == text.len() {
            Ok(text)
        } else {
            Ok(trimmed.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SearchQuery, UserSearch};
    use crate::Validated;

    /// Both searches print their length in `Debug`, and none of the text.
    #[test]
    fn a_search_prints_its_length_alone() {
        let query = Validated::<SearchQuery>::parse("salary review").expect("a query");
        assert_eq!(format!("{query:?}"), "13 chars");
        let search = Validated::<UserSearch>::parse(" ada ").expect("a search");
        assert_eq!(format!("{search:?}"), "3 chars");
    }

    /// A message search keeps its text as typed, and refuses a blank one,
    /// from the box as from the bytes.
    #[test]
    fn a_message_search_is_never_blank() {
        let query = Validated::<SearchQuery>::parse(" bud ").expect("a word");
        assert_eq!(query.as_str(), " bud ");
        for blank in ["", " ", "\t\n"] {
            assert!(Validated::<SearchQuery>::parse(blank).is_err(), "{blank:?}");
            let bytes = postcard::to_allocvec(blank).expect("a string encodes");
            assert!(
                postcard::from_bytes::<Validated<SearchQuery>>(&bytes).is_err(),
                "{blank:?}"
            );
        }
        let bytes = postcard::to_allocvec(" bud ").expect("a string encodes");
        assert_eq!(
            postcard::to_allocvec(&query).expect("a query encodes"),
            bytes
        );
        assert_eq!(
            serde_json::to_string(&query).expect("a query encodes"),
            "\" bud \""
        );
        let refused = serde_json::from_str::<Validated<SearchQuery>>("\" \"")
            .expect_err("blank")
            .to_string();
        assert!(refused.starts_with("a search is never blank"), "{refused}");
    }

    /// A search keeps its trimmed text, and refuses one shorter than two
    /// characters, counted as characters rather than bytes.
    #[test]
    fn a_search_is_two_characters_or_none() {
        assert_eq!(
            Validated::<UserSearch>::parse("  al ").map(Validated::into_string),
            Ok("al".to_owned())
        );
        for short in ["", "   ", "a", " é "] {
            assert!(Validated::<UserSearch>::parse(short).is_err(), "{short:?}");
        }
        assert!(Validated::<UserSearch>::parse("éé").is_ok());
    }

    /// A user search travels as its bare text, and the bytes of a text too
    /// short to search are refused.
    #[test]
    fn a_user_search_travels_as_its_text_and_refuses_a_short_one() {
        let search = Validated::<UserSearch>::parse(" ab ").expect("two characters");
        let bytes = postcard::to_allocvec(&search).expect("a search encodes");
        assert_eq!(
            bytes,
            postcard::to_allocvec("ab").expect("a string encodes")
        );
        assert_eq!(
            postcard::from_bytes::<Validated<UserSearch>>(&bytes),
            Ok(search)
        );
        for short in ["", "a", " a "] {
            let bytes = postcard::to_allocvec(short).expect("a string encodes");
            assert!(
                postcard::from_bytes::<Validated<UserSearch>>(&bytes).is_err(),
                "{short:?}"
            );
        }
    }
}
