//! The reactions of a message
//! (`specs/5-Murmure/features/ambre-spec-murmure-reactions.md`).
//!
//! A message holds one flat `reactions` map, created empty with it
//! (`MURMURE-256`). Each entry is keyed by its member and its emoji, so a
//! member writes and removes their own key alone (`MURMURE-006`). Two members
//! reacting with one emoji at once write two keys, and both stay
//! (`MURMURE-007`).

use ambre_bus::{Emoji, Reaction};
use ambre_core::UserId;
use loro::{Container, ContainerID, Index, LoroDoc, LoroMap, ValueOrContainer};
use unicode_segmentation::UnicodeSegmentation;

use crate::schema::{MESSAGE_REACTIONS, MESSAGES};

/// The longest emoji a key takes, in bytes. A family or a flag sequence of
/// Unicode stays well below it.
const EMOJI_MAX_BYTES: usize = 64;

/// The key of the reaction of `user` with `emoji`.
///
/// The user comes first: a `UserId` is written without a space, so the first
/// space ends it, whatever the emoji holds.
pub(crate) fn key(user: UserId, emoji: &Emoji) -> String {
    format!("{user} {}", emoji.as_str())
}

/// The member a reaction key names, `None` for a key no build writes.
pub(crate) fn user_of(key: &str) -> Option<UserId> {
    let (user, _) = key.split_once(' ')?;
    UserId::parse_str(user).ok()
}

/// Whether `emoji` is one emoji: one grapheme, with no space or control
/// character, inside [`EMOJI_MAX_BYTES`].
pub(crate) fn is_emoji(emoji: &str) -> bool {
    emoji.len() <= EMOJI_MAX_BYTES
        && emoji.graphemes(true).count() == 1
        && !emoji.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The reactions of one message, one per emoji in emoji order, each member
/// once in identifier order. An unreadable key is skipped.
pub(crate) fn read(message: &LoroMap) -> Vec<Reaction> {
    let Some(ValueOrContainer::Container(Container::Map(map))) = message.get(MESSAGE_REACTIONS)
    else {
        return Vec::new();
    };
    // The written identifier sorts as its bytes do.
    let mut entries: Vec<(Emoji, String, UserId)> = Vec::new();
    map.for_each(|key, _| {
        let Some((written, emoji)) = key.split_once(' ') else {
            return;
        };
        if let Ok(user) = UserId::parse_str(written) {
            entries.push((Emoji(emoji.to_owned()), written.to_owned(), user));
        }
    });
    entries.sort_unstable_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let mut reactions: Vec<Reaction> = Vec::new();
    for (emoji, _, user) in entries {
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
            key.as_str() == MESSAGE_REACTIONS
                && matches!(list, ContainerID::Root { name, .. } if name.as_str() == MESSAGES)
        }
        _ => false,
    }
}
