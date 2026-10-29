//! The `@` list of the composer (the Murmure mentions spec).
//!
//! An `@` that starts a word opens the list, and what follows it up to the
//! caret filters it. `@channel` comes first, then the members of the channel
//! by name (`MURMURE-551`). A pick writes the name into the draft, and a
//! mention over it rides with the message (`MURMURE-552`).

use ambre_bus::{MentionTarget, UserId};
use ambre_core::{ByteCount, ByteOffset, ViewPoint};

use crate::Region;

/// The word an `@channel` writes, after its `@`.
pub const AT_CHANNEL: &str = "channel";

ambre_core::define_index! {
    /// The place of an entry in the `@` list, `@channel` first.
    pub struct OfferIndex(u32);
    /// A number of entries of the `@` list.
    pub struct OfferCount;
    /// The signed gap between two entries of the `@` list.
    pub struct OfferDelta(i64);
    /// A run of entries of the `@` list.
    pub struct OfferSpan;
    /// A list indexed by the entries of the `@` list.
    pub struct OfferVec;
}

impl OfferIndex {
    /// The entry after this one in a list of `len` entries, the first after
    /// the last.
    pub fn after(self, len: OfferCount) -> Self {
        let next = self.next();
        if next < len.as_end() {
            next
        } else {
            Self::ZERO
        }
    }

    /// The entry before this one in a list of `len` entries, the last
    /// before the first.
    pub fn before(self, len: OfferCount) -> Self {
        self.prev()
            .unwrap_or_else(|| len.as_end().prev().unwrap_or(Self::ZERO))
    }

    /// This entry, or the last of a list of `len` entries when it is past
    /// it.
    pub fn within(self, len: OfferCount) -> Self {
        self.min(len.as_end().prev().unwrap_or(Self::ZERO))
    }
}

/// One entry of the list: what a pick names, and the name it writes.
///
/// `Debug` prints the length of the name alone: a member chose it.
#[derive(Clone, PartialEq, Eq)]
pub struct Offer {
    pub target: MentionTarget,
    pub name: ambre_core::DisplayName,
}

impl std::fmt::Debug for Offer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Offer")
            .field("target", &self.target)
            .field("name", &ByteCount::from_len(self.name.as_str().len()))
            .finish()
    }
}

impl Offer {
    /// The text a pick writes over the query: `@` and the name.
    pub fn written(&self) -> String {
        format!("@{}", self.name.as_str())
    }
}

/// What the page draws of an open list: the names, the one Enter picks, and
/// the caret it hangs from (`MURMURE-555`).
///
/// `Debug` counts the entries alone: each one names a member.
#[derive(Clone, PartialEq)]
pub struct MentionList {
    /// Each entry as the list shows it, `@channel` first.
    pub entries: OfferVec<String>,
    /// The entry Enter or Tab picks.
    pub chosen: OfferIndex,
    /// The top-left of the caret, in pixels of the region's canvas.
    pub at: ViewPoint,
    /// The region the list hangs from.
    pub region: Region,
}

impl std::fmt::Debug for MentionList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MentionList")
            .field("entries", &self.entries.len())
            .field("chosen", &self.chosen)
            .field("at", &self.at)
            .field("region", &self.region)
            .finish()
    }
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
fn matches(name: &ambre_core::DisplayName, query: &str) -> bool {
    let query = query.to_lowercase();
    name.as_str()
        .to_lowercase()
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
        .filter(|(user, name)| Some(*user) != me && matches(name, query))
        .collect();
    kept.sort_by_cached_key(|(_, name)| name.as_str().to_lowercase());
    // The word is a valid display name, so the offer is always built; a
    // test holds it first in the list.
    let channel = AT_CHANNEL
        .starts_with(&query.to_lowercase())
        .then(|| ambre_core::DisplayName::new(AT_CHANNEL).ok())
        .flatten()
        .map(|name| Offer {
            target: MentionTarget::Channel,
            name,
        });
    channel
        .into_iter()
        .chain(kept.into_iter().map(|(user, name)| Offer {
            target: MentionTarget::User(*user),
            name: name.clone(),
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The debug form of an entry and of the list names no member: it
    /// prints a length and a count.
    #[test]
    fn an_offer_debugs_its_name_length_alone() {
        let name = ambre_core::DisplayName::new("Secretary").expect("a name");
        let members = [(UserId::new(), name)];
        let found = offers("sec", &members, None);
        let shown = format!("{found:?}");
        assert!(!shown.contains("Secretary"), "{shown}");
        assert!(shown.contains('9'), "{shown}");
        let list = MentionList {
            entries: found.iter().map(Offer::written).collect(),
            chosen: OfferIndex::ZERO,
            at: ViewPoint::default(),
            region: Region::Channel,
        };
        let shown = format!("{list:?}");
        assert!(!shown.contains("Secretary"), "{shown}");
    }

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

    /// The arrows go round the list, and a list that shrank keeps the
    /// choice on its last entry.
    #[test]
    fn the_choice_goes_round_the_list_and_stays_on_it() {
        let three = OfferCount::new(3);
        let (first, last) = (OfferIndex::ZERO, OfferIndex::new(2));
        assert_eq!(first.after(three), OfferIndex::new(1));
        assert_eq!(last.after(three), first);
        assert_eq!(first.before(three), last);
        assert_eq!(last.before(three), OfferIndex::new(1));
        assert_eq!(OfferIndex::new(5).within(three), last);
        assert_eq!(OfferIndex::new(1).within(three), OfferIndex::new(1));
    }
}
