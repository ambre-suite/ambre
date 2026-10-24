//! The reactions of a message
//! (`specs/5-Murmure/features/ambre-spec-murmure-reactions.md`).
//!
//! A message holds one flat `reactions` map, created empty with it
//! (`MURMURE-256`). Each entry is keyed by its member and its emoji, so a
//! member writes and removes their own key alone (`MURMURE-006`). Two members
//! reacting with one emoji at once write two keys, and both stay
//! (`MURMURE-007`).

use ambre_bus::{Emoji, Reaction};
use ambre_core::{ByteSize, UserId};
use loro::{Container, ContainerID, Index, LoroDoc, ValueOrContainer};
use unicode_segmentation::UnicodeSegmentation;

use crate::schema::{Fields, MessageKey, Root};

/// The longest emoji a key takes, in bytes. A family or a flag sequence of
/// Unicode stays well below it.
const EMOJI_MAX_SIZE: ByteSize = ByteSize(64);

/// The key of one reaction in the `reactions` map of a message: its member
/// and its emoji (`MURMURE-006`).
///
/// The user comes first: a `UserId` is written without a space, so the first
/// space ends it, whatever the emoji holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactionKey(String);

impl ReactionKey {
    /// The key of the reaction of `user` with `emoji`.
    pub(crate) fn of(user: UserId, emoji: &Emoji) -> Self {
        Self(format!("{user} {}", emoji.as_str()))
    }

    /// The key as the map stores it, for the call into the map.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The member and the emoji a stored key names, `None` for a key no
    /// build writes.
    pub(crate) fn read(stored: &str) -> Option<(UserId, Emoji)> {
        let (user, emoji) = stored.split_once(' ')?;
        Some((UserId::parse_str(user).ok()?, Emoji::from(emoji)))
    }
}

/// Whether `emoji` is one emoji: one grapheme, with no space or control
/// character, inside [`EMOJI_MAX_SIZE`].
pub(crate) fn is_emoji(emoji: &Emoji) -> bool {
    let emoji = emoji.as_str();
    ByteSize::of_len(emoji.len()) <= EMOJI_MAX_SIZE
        && emoji.graphemes(true).count() == 1
        && !emoji.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The reactions of one message, one per emoji in emoji order, each member
/// once in identifier order. An unreadable key is skipped.
pub(crate) fn read(message: &Fields<MessageKey>) -> Vec<Reaction> {
    let Some(ValueOrContainer::Container(Container::Map(map))) = message.get(MessageKey::Reactions)
    else {
        return Vec::new();
    };
    let mut entries: Vec<(Emoji, UserId)> = Vec::new();
    map.for_each(|key, _| {
        if let Some((user, emoji)) = ReactionKey::read(key) {
            entries.push((emoji, user));
        }
    });
    // The bytes of an identifier sort as its written form does.
    entries.sort_unstable_by(|a, b| (&a.0, a.1.as_bytes()).cmp(&(&b.0, b.1.as_bytes())));
    let mut reactions: Vec<Reaction> = Vec::new();
    for (emoji, user) in entries {
        match reactions.last_mut() {
            Some(last) if last.emoji == emoji => last.users.push(user),
            _ => reactions.push(Reaction {
                emoji,
                users: vec![user],
            }),
        }
    }
    reactions
}

/// Whether `id` is the `reactions` map of a message `doc` holds: the map
/// under that key, in a message of the `messages` list.
pub(crate) fn is_map(doc: &LoroDoc, id: &ContainerID) -> bool {
    let Some(path) = doc.get_path_to_container(id) else {
        return false;
    };
    match path.as_slice() {
        [(list, _), (_, Index::Seq(_)), (_, Index::Key(key))] => {
            MessageKey::parse(key) == Some(MessageKey::Reactions)
                && matches!(list, ContainerID::Root { name, .. } if Root::parse(name) == Some(Root::Messages))
        }
        _ => false,
    }
}
