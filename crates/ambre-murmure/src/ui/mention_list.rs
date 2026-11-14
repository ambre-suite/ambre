//! The `@` list of the composer (the Murmure mentions spec).
//!
//! An `@` that starts a word opens the list, and what follows it up to the
//! caret filters it. `@channel` comes first, then the members of the channel
//! by name (`MURMURE-551`). A pick writes the name into the draft, and a
//! mention over it rides with the message (`MURMURE-552`).

use crate::message::MentionTarget;
use ambre_core::id::{Id, User};
use ambre_core::index::{Axis, Circular, IdxVec, Len, Pos};
use ambre_core::{Byte, Point, Px, View};

use crate::Region;

/// The word an `@channel` writes, after its `@`.
pub const AT_CHANNEL: &str = "channel";

/// The place of an entry in the `@` list, `@channel` first.
#[derive(PartialEq, Eq)]
pub enum MentionEntry {}

impl Axis for MentionEntry {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Offer");
}

/// The arrow keys wrap the choice: past the last entry comes the first.
impl Circular for MentionEntry {}

/// One entry of the list: what a pick names, and the name it writes.
///
/// `Debug` prints the length of the name alone: a member chose it.
#[derive(Clone, PartialEq, Eq)]
pub struct Offer {
    pub target: MentionTarget,
    pub name: ambre_core::Validated<ambre_core::DisplayName>,
}

impl std::fmt::Debug for Offer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Offer")
            .field("target", &self.target)
            .field("name", &Len::<Byte>::of(self.name.as_str().len()))
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
    pub entries: IdxVec<MentionEntry, String>,
    /// The entry Enter or Tab picks.
    pub chosen: Pos<MentionEntry>,
    /// The top-left of the caret, in pixels of the region's canvas.
    pub at: Point<Px<View>>,
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
pub fn query_at(text: &str, caret: Pos<Byte>) -> Option<(Pos<Byte>, &str)> {
    let before = text.get(..caret.to_usize())?;
    let at = before.rfind(|c: char| c == '@' || c.is_whitespace())?;
    if !before[at..].starts_with('@') {
        return None;
    }
    let starts_word = before[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    starts_word.then(|| (Pos::<Byte>::from_usize(at), &before[at + 1..]))
}

/// Whether a word of `name` starts with `query`, case folded.
fn matches(name: &ambre_core::Validated<ambre_core::DisplayName>, query: &str) -> bool {
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
    members: &[(Id<User>, ambre_core::Validated<ambre_core::DisplayName>)],
    me: Option<Id<User>>,
) -> Vec<Offer> {
    let mut kept: Vec<&(Id<User>, ambre_core::Validated<ambre_core::DisplayName>)> = members
        .iter()
        .filter(|(user, name)| Some(*user) != me && matches(name, query))
        .collect();
    kept.sort_by_cached_key(|(_, name)| name.as_str().to_lowercase());
    // The word is a valid display name, so the offer is always built; a
    // test holds it first in the list.
    let channel = AT_CHANNEL
        .starts_with(&query.to_lowercase())
        .then(|| ambre_core::Validated::<ambre_core::DisplayName>::parse(AT_CHANNEL).ok())
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
    use ambre_core::index::Delta;

    /// The debug form of an entry and of the list names no member: it
    /// prints a length and a count.
    #[test]
    fn an_offer_debugs_its_name_length_alone() {
        let name =
            ambre_core::Validated::<ambre_core::DisplayName>::parse("Secretary").expect("a name");
        let members = [(Id::<User>::mint(), name)];
        let found = offers("sec", &members, None);
        let shown = format!("{found:?}");
        assert!(!shown.contains("Secretary"), "{shown}");
        assert!(shown.contains("name: ByteCount(9)"), "{shown}");
        let list = MentionList {
            entries: found.iter().map(Offer::written).collect(),
            chosen: Pos::<MentionEntry>::ZERO,
            at: Point::default(),
            region: Region::Channel,
        };
        let shown = format!("{list:?}");
        assert!(!shown.contains("Secretary"), "{shown}");
        assert!(shown.contains("entries: OfferCount(1)"), "{shown}");
    }

    /// An `@` opens a query only where it starts a word, and the query runs
    /// to the caret until a space ends it (`MURMURE-551`).
    #[test]
    fn an_at_that_starts_a_word_opens_a_query_up_to_the_caret() {
        assert_eq!(
            query_at("@", Pos::<Byte>::new(1)),
            Some((Pos::<Byte>::new(0), ""))
        );
        assert_eq!(
            query_at("hi @bo", Pos::<Byte>::new(6)),
            Some((Pos::<Byte>::new(3), "bo"))
        );
        assert_eq!(
            query_at("hi @bo", Pos::<Byte>::new(5)),
            Some((Pos::<Byte>::new(3), "b"))
        );
        assert_eq!(
            query_at("é\n@x", Pos::<Byte>::from_usize("é\n@x".len())),
            Some((Pos::<Byte>::new(3), "x"))
        );
        assert_eq!(
            query_at("mail@bo", Pos::<Byte>::new(7)),
            None,
            "inside a word"
        );
        assert_eq!(
            query_at("@bo b", Pos::<Byte>::new(5)),
            None,
            "a space ended it"
        );
        assert_eq!(query_at("hi", Pos::<Byte>::new(2)), None);
        assert_eq!(
            query_at("é", Pos::<Byte>::new(1)),
            None,
            "off a char boundary"
        );
    }

    /// `@channel` leads while the query can still spell it. Members follow
    /// by name, matched on the start of any word, and never the writer.
    #[test]
    fn the_channel_leads_and_members_follow_by_name() {
        let (alice, bob, carol) = (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
        let members = vec![
            (
                carol,
                ambre_core::Validated::<ambre_core::DisplayName>::parse("carol Chen").unwrap(),
            ),
            (
                bob,
                ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap(),
            ),
            (
                alice,
                ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
            ),
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
        let three = Len::<MentionEntry>::new(3);
        let (first, last) = (Pos::<MentionEntry>::ZERO, Pos::<MentionEntry>::new(2));
        let (down, up) = (Delta::new(1), Delta::new(-1));
        assert_eq!(
            first.wrapped(down, three),
            Some(Pos::<MentionEntry>::new(1))
        );
        assert_eq!(last.wrapped(down, three), Some(first));
        assert_eq!(first.wrapped(up, three), Some(last));
        assert_eq!(last.wrapped(up, three), Some(Pos::<MentionEntry>::new(1)));
        assert_eq!(Pos::<MentionEntry>::new(5).within(three), last);
        assert_eq!(
            Pos::<MentionEntry>::new(1).within(three),
            Pos::<MentionEntry>::new(1)
        );
    }
}
