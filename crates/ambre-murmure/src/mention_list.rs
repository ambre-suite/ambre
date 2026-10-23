//! The `@` list of the composer (the Murmure mentions spec).
//!
//! An `@` that starts a word opens the list, and what follows it up to the
//! caret filters it. `@channel` comes first, then the members of the channel
//! by name (`MURMURE-551`). A pick writes the name into the draft, and a
//! mention over it rides with the message (`MURMURE-552`).

use ambre_bus::{MentionTarget, UserId};
use ambre_core::{ByteOffset, ViewPoint};

use crate::Region;

/// The word an `@channel` writes, after its `@`.
pub const AT_CHANNEL: &str = "channel";

/// One entry of the list: what a pick names, and the name it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub target: MentionTarget,
    pub name: String,
}

impl Offer {
    /// The text a pick writes over the query: `@` and the name.
    pub fn written(&self) -> String {
        format!("@{}", self.name)
    }
}

/// What the page draws of an open list: the names, the one Enter picks, and
/// the caret it hangs from (`MURMURE-555`).
#[derive(Debug, Clone, PartialEq)]
pub struct MentionList {
    /// Each entry as the list shows it, `@channel` first.
    pub entries: Vec<String>,
    /// The entry Enter or Tab picks.
    pub chosen: usize,
    /// The top-left of the caret, in pixels of the region's canvas.
    pub at: ViewPoint,
    /// The region the list hangs from.
    pub region: Region,
}

/// The query an `@` opens before the byte `caret` of `text`: the offset of
/// its `@`, and what follows it. The `@` starts the text or follows a space,
/// and no space comes after it.
pub fn query_at(text: &str, caret: ByteOffset) -> Option<(ByteOffset, &str)> {
    let before = text.get(..caret.get())?;
    let at = before.rfind(|c: char| c == '@' || c.is_whitespace())?;
    if !before[at..].starts_with('@') {
        return None;
    }
    let starts_word = before[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    starts_word.then(|| (ByteOffset::new(at), &before[at + 1..]))
}

/// Whether a word of `name` starts with `query`, case folded.
fn matches(name: &str, query: &str) -> bool {
    let query = query.to_lowercase();
    name.to_lowercase()
        .split_whitespace()
        .any(|word| word.starts_with(&query))
}

/// The entries `query` keeps: `@channel` first, then each member in name
/// order. `me` is never offered.
pub fn offers(
    query: &str,
    members: &[(UserId, ambre_core::DisplayName)],
    me: Option<UserId>,
) -> Vec<Offer> {
    let mut kept: Vec<&(UserId, ambre_core::DisplayName)> = members
        .iter()
        .filter(|(user, name)| Some(*user) != me && matches(name.as_str(), query))
        .collect();
    kept.sort_by_cached_key(|(_, name)| name.as_str().to_lowercase());
    let channel = AT_CHANNEL
        .starts_with(&query.to_lowercase())
        .then(|| Offer {
            target: MentionTarget::Channel,
            name: AT_CHANNEL.to_owned(),
        });
    channel
        .into_iter()
        .chain(kept.into_iter().map(|(user, name)| Offer {
            target: MentionTarget::User(*user),
            name: name.as_str().to_owned(),
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `@` opens a query only where it starts a word, and the query runs
    /// to the caret until a space ends it (`MURMURE-551`).
    #[test]
    fn an_at_that_starts_a_word_opens_a_query_up_to_the_caret() {
        assert_eq!(
            query_at("@", ByteOffset::new(1)),
            Some((ByteOffset::new(0), ""))
        );
        assert_eq!(
            query_at("hi @bo", ByteOffset::new(6)),
            Some((ByteOffset::new(3), "bo"))
        );
        assert_eq!(
            query_at("hi @bo", ByteOffset::new(5)),
            Some((ByteOffset::new(3), "b"))
        );
        assert_eq!(
            query_at("é\n@x", ByteOffset::new("é\n@x".len())),
            Some((ByteOffset::new(3), "x"))
        );
        assert_eq!(
            query_at("mail@bo", ByteOffset::new(7)),
            None,
            "inside a word"
        );
        assert_eq!(
            query_at("@bo b", ByteOffset::new(5)),
            None,
            "a space ended it"
        );
        assert_eq!(query_at("hi", ByteOffset::new(2)), None);
        assert_eq!(
            query_at("é", ByteOffset::new(1)),
            None,
            "off a char boundary"
        );
    }

    /// `@channel` leads while the query can still spell it. Members follow
    /// by name, matched on the start of any word, and never the writer.
    #[test]
    fn the_channel_leads_and_members_follow_by_name() {
        let (alice, bob, carol) = (UserId::new(), UserId::new(), UserId::new());
        let members = vec![
            (carol, ambre_core::DisplayName::new("carol Chen").unwrap()),
            (bob, ambre_core::DisplayName::new("Bob").unwrap()),
            (alice, ambre_core::DisplayName::new("Alice").unwrap()),
        ];
        let names = |query, me| -> Vec<String> {
            offers(query, &members, me)
                .into_iter()
                .map(|o| o.written())
                .collect()
        };
        assert_eq!(names("", Some(alice)), ["@channel", "@Bob", "@carol Chen"]);
        assert_eq!(names("C", None), ["@channel", "@carol Chen"]);
        assert_eq!(names("ch", None), ["@channel", "@carol Chen"]);
        assert_eq!(names("cha", None), ["@channel"]);
        assert_eq!(names("b", None), ["@Bob"]);
        assert_eq!(names("zed", None), Vec::<String>::new());
        assert_eq!(offers("", &members, None)[0].target, MentionTarget::Channel);
    }
}
