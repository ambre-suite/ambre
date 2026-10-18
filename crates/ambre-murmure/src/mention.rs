//! The `mention` marks of a message body (`features/ambre-spec-murmure-mentions.md`).
//!
//! A mark carries a user identifier, or the channel identifier for an
//! `@channel` (`MURMURE-008`). It expands to nothing, so text typed beside
//! it stays out of the mention. The marks are the truth: nothing here parses
//! the text (`MURMURE-103`).

use ambre_bus::{Mention, MentionTarget};
use ambre_core::{ChannelId, UserId};
use loro::{ExpandType, LoroDoc, LoroText, LoroValue, StyleConfig, StyleConfigMap, TextDelta};

use crate::error::MurmureError;
use crate::schema::MENTION;

/// The value a mark stores for `target`: the user, or the channel itself.
fn value_of(target: MentionTarget, channel: ChannelId) -> String {
    match target {
        MentionTarget::User(user) => user.to_string(),
        MentionTarget::Channel => channel.to_string(),
    }
}

/// What a stored mark names, or `None` for a value that names neither a user
/// nor this channel.
fn target_of(value: &LoroValue, channel: ChannelId) -> Option<MentionTarget> {
    let LoroValue::String(value) = value else {
        return None;
    };
    if **value == channel.to_string() {
        return Some(MentionTarget::Channel);
    }
    UserId::parse_str(value).ok().map(MentionTarget::User)
}

/// Whether every range of `mentions` holds chars of `text`.
pub(crate) fn fits(mentions: &[Mention], text: &str) -> bool {
    let len = text.chars().count();
    mentions
        .iter()
        .all(|m| !m.range.is_empty() && m.range.end <= len)
}

/// Mark each mention of `mentions` over `content` (`MURMURE-100`). The
/// caller checked them with [`fits`].
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a mark.
pub(crate) fn write(
    doc: &LoroDoc,
    content: &LoroText,
    mentions: &[Mention],
    channel: ChannelId,
) -> Result<(), MurmureError> {
    if mentions.is_empty() {
        return Ok(());
    }
    // The style config is local to this document and read at the mark: a
    // receiver takes the expansion from the op itself.
    let mut styles = StyleConfigMap::default_rich_text_config();
    styles.insert(
        MENTION.into(),
        StyleConfig {
            expand: ExpandType::None,
        },
    );
    doc.config_text_style(styles);
    for mention in mentions {
        content.mark(
            mention.range.clone(),
            MENTION,
            value_of(mention.target, channel),
        )?;
    }
    Ok(())
}

/// The mentions `content` carries, in text order (`MURMURE-103`). A read,
/// never a write (`MURMURE-152`).
pub(crate) fn read(content: &LoroText, channel: ChannelId) -> Vec<Mention> {
    let mut mentions = Vec::new();
    let mut at = 0;
    for span in content.to_delta() {
        let TextDelta::Insert { insert, attributes } = span else {
            continue;
        };
        let len = insert.chars().count();
        let target = attributes
            .as_ref()
            .and_then(|attributes| attributes.get(MENTION))
            .and_then(|value| target_of(value, channel));
        if let Some(target) = target {
            mentions.push(Mention {
                target,
                range: at..at + len,
            });
        }
        at += len;
    }
    mentions
}

/// Whether `mentions` names `me`, or the channel and so every member
/// (`MURMURE-233`).
pub(crate) fn names(mentions: &[Mention], me: UserId) -> bool {
    mentions.iter().any(|m| match m.target {
        MentionTarget::User(user) => user == me,
        MentionTarget::Channel => true,
    })
}
