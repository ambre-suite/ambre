//! The activity summary an update may carry, and the counters it moves
//! (`ambre-spec-residency.md section 3`, `SYNC-113`).
//!
//! The summary is written by the application that writes the update, and is
//! sealed with it. The Sync worker reads it with no document, so a document not
//! held in memory still moves its counters (`RESID-016`).

use serde::{Deserialize, Serialize};

use crate::id::UserId;

/// The most members one summary names. A message that names more names the
/// channel, as an `@channel` does.
pub const MAX_MENTIONS: usize = 64;

/// The longest anchor, in bytes. An anchor is an application key, such as a
/// thread root, never content.
pub const MAX_ANCHOR_LEN: usize = 64;

crate::define_flag! {
    /// Whether a document saw an update past the reader's cursor, counted or
    /// not (`RESID-015`).
    pub enum Activity {
        /// Nothing happened.
        Quiet,
        /// Something happened.
        Active,
    }

    /// Whether an update names every member, as an `@channel` does, or only
    /// the members it lists.
    pub enum MentionScope {
        /// Only the members the update lists.
        Named,
        /// Every member.
        All,
    }
}

/// What kind of activity an update is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// New content, such as a message.
    Content,
    /// A reaction to content.
    Reaction,
    /// An edit of earlier content.
    Edit,
    /// Anything else an application counts as activity.
    Other,
}

impl ActivityKind {
    /// The byte that carries the kind in a summary.
    const fn byte(self) -> u8 {
        match self {
            Self::Content => 0,
            Self::Reaction => 1,
            Self::Edit => 2,
            Self::Other => 3,
        }
    }

    /// The kind a byte names, `None` for a byte no build writes.
    const fn of(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Content),
            1 => Some(Self::Reaction),
            2 => Some(Self::Edit),
            3 => Some(Self::Other),
            _ => None,
        }
    }
}

/// An application key a summary points at, opaque to the foundation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityAnchor(Vec<u8>);

impl ActivityAnchor {
    /// An anchor of at most [`MAX_ANCHOR_LEN`] bytes, `None` past it.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        (bytes.len() <= MAX_ANCHOR_LEN).then_some(Self(bytes))
    }

    /// The application key.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// What an update says of itself, for a client that does not hold its
/// document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivitySummary {
    /// The kind of activity.
    pub kind: ActivityKind,
    /// The members the update names, at most [`MAX_MENTIONS`].
    pub mentions: Vec<UserId>,
    /// Whether the update names every member, as an `@channel` does.
    pub mentions_all: MentionScope,
    /// The application key it points at, such as a thread root.
    pub anchor: Option<ActivityAnchor>,
}

/// Why a summary could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SummaryError {
    /// The bytes end inside the summary.
    #[error("an activity summary is shorter than its parts")]
    Short,
    /// A kind no build writes.
    #[error("an activity summary names an unknown kind {0}")]
    UnknownKind(u8),
    /// More mentions than one summary carries.
    #[error("an activity summary names more than {MAX_MENTIONS} members")]
    TooManyMentions,
    /// An anchor past its bound.
    #[error("an activity summary anchor is longer than {MAX_ANCHOR_LEN} bytes")]
    LongAnchor,
}

/// The flag bit of a summary that names every member.
const FLAG_ALL: u8 = 0b01;

impl ActivitySummary {
    /// The summary as bytes: the kind, the flag byte, the mention count, each
    /// member, then the anchor length and the anchor.
    ///
    /// # Errors
    ///
    /// [`SummaryError::TooManyMentions`] past [`MAX_MENTIONS`]. The anchor is
    /// bounded by its own type.
    pub fn encode(&self) -> Result<Vec<u8>, SummaryError> {
        let count = u8::try_from(self.mentions.len())
            .ok()
            .filter(|count| usize::from(*count) <= MAX_MENTIONS)
            .ok_or(SummaryError::TooManyMentions)?;
        let anchor = self
            .anchor
            .as_ref()
            .map_or(&[][..], ActivityAnchor::as_bytes);
        let anchor_len = u8::try_from(anchor.len()).map_err(|_| SummaryError::LongAnchor)?;
        let mut out = Vec::with_capacity(4 + 16 * self.mentions.len() + anchor.len());
        out.push(self.kind.byte());
        out.push(if self.mentions_all == MentionScope::All {
            FLAG_ALL
        } else {
            0
        });
        out.push(count);
        for user in &self.mentions {
            out.extend_from_slice(user.as_bytes());
        }
        out.push(anchor_len);
        out.extend_from_slice(anchor);
        Ok(out)
    }

    /// Reads a summary from the head of `bytes`, returning it and the bytes
    /// that follow it.
    ///
    /// # Errors
    ///
    /// Every [`SummaryError`]: a summary past its bounds is refused, never
    /// truncated.
    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), SummaryError> {
        let (head, rest) = bytes.split_first_chunk::<3>().ok_or(SummaryError::Short)?;
        let [kind, flags, count] = *head;
        let kind = ActivityKind::of(kind).ok_or(SummaryError::UnknownKind(kind))?;
        let count = usize::from(count);
        if count > MAX_MENTIONS {
            return Err(SummaryError::TooManyMentions);
        }
        let (users, rest) = rest
            .split_at_checked(count * 16)
            .ok_or(SummaryError::Short)?;
        let mentions = users
            .chunks_exact(16)
            .filter_map(|chunk| <[u8; 16]>::try_from(chunk).ok())
            .map(UserId::from_bytes)
            .collect();
        let (&anchor_len, rest) = rest.split_first().ok_or(SummaryError::Short)?;
        let anchor_len = usize::from(anchor_len);
        if anchor_len > MAX_ANCHOR_LEN {
            return Err(SummaryError::LongAnchor);
        }
        let (anchor, rest) = rest
            .split_at_checked(anchor_len)
            .ok_or(SummaryError::Short)?;
        let summary = Self {
            kind,
            mentions,
            mentions_all: MentionScope::from_bool(flags & FLAG_ALL != 0),
            anchor: (!anchor.is_empty()).then(|| ActivityAnchor(anchor.to_vec())),
        };
        Ok((summary, rest))
    }
}

crate::define_tally! {
    /// Updates of kind content.
    pub struct ContentUpdates;
    /// Updates of kind reaction.
    pub struct ReactionUpdates;
    /// Updates of kind edit.
    pub struct EditUpdates;
    /// Updates of any other kind.
    pub struct OtherUpdates;
    /// Updates that name this user, or every member.
    pub struct MentionUpdates;
}

/// The activity a client counts for one document it does not hold
/// (`RESID-004`). A few dozen bytes, whatever the document's size.
///
/// Each counter has its own type, so a reaction is never counted as content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActivityCounters {
    /// Updates of kind content.
    pub content: ContentUpdates,
    /// Updates of kind reaction.
    pub reactions: ReactionUpdates,
    /// Updates of kind edit.
    pub edits: EditUpdates,
    /// Updates of any other kind.
    pub other: OtherUpdates,
    /// Updates that name this user, or every member.
    pub mentions: MentionUpdates,
    /// Whether something happened, counted or not (`RESID-015`).
    pub active: Activity,
}

impl ActivityCounters {
    /// Moves the counters by one update (`RESID-014`). An update with no
    /// summary marks the document active and counts nothing (`RESID-015`).
    /// With no `me`, only a mention of every member counts.
    pub fn advance(&mut self, summary: Option<&ActivitySummary>, me: Option<UserId>) {
        self.active = Activity::Active;
        let Some(summary) = summary else {
            return;
        };
        match summary.kind {
            ActivityKind::Content => self.content = self.content.incremented(),
            ActivityKind::Reaction => self.reactions = self.reactions.incremented(),
            ActivityKind::Edit => self.edits = self.edits.incremented(),
            ActivityKind::Other => self.other = self.other.incremented(),
        }
        if summary.mentions_all == MentionScope::All
            || me.is_some_and(|me| summary.mentions.contains(&me))
        {
            self.mentions = self.mentions.incremented();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(byte: u8) -> UserId {
        UserId::from_bytes([byte; 16])
    }

    fn summary() -> ActivitySummary {
        ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![user(1), user(2)],
            mentions_all: MentionScope::Named,
            anchor: ActivityAnchor::new(b"root".to_vec()),
        }
    }

    #[test]
    fn a_summary_reads_back_and_leaves_what_follows() {
        let mut bytes = summary().encode().expect("within bounds");
        bytes.extend_from_slice(b"delta");
        let (read, rest) = ActivitySummary::decode(&bytes).expect("a summary");
        assert_eq!(read, summary());
        assert_eq!(rest, b"delta");
    }

    #[test]
    fn a_summary_past_its_bounds_is_refused() {
        let crowd = ActivitySummary {
            mentions: (0..=64).map(user).collect(),
            ..summary()
        };
        assert_eq!(crowd.encode(), Err(SummaryError::TooManyMentions));
        assert!(ActivityAnchor::new(vec![0; MAX_ANCHOR_LEN + 1]).is_none());
        let mut bytes = summary().encode().expect("within bounds");
        bytes.truncate(bytes.len() - 1);
        assert_eq!(ActivitySummary::decode(&bytes), Err(SummaryError::Short));
        assert_eq!(
            ActivitySummary::decode(&[9, 0, 0, 0]),
            Err(SummaryError::UnknownKind(9))
        );
        assert_eq!(
            ActivitySummary::decode(&[0, 0, 65]),
            Err(SummaryError::TooManyMentions)
        );
    }

    #[test]
    fn an_update_with_a_summary_moves_its_counters() {
        let mut counters = ActivityCounters::default();
        counters.advance(Some(&summary()), Some(user(2)));
        counters.advance(
            Some(&ActivitySummary {
                kind: ActivityKind::Reaction,
                mentions: Vec::new(),
                mentions_all: MentionScope::All,
                anchor: None,
            }),
            None,
        );
        counters.advance(Some(&summary()), Some(user(9)));
        assert_eq!(
            counters,
            ActivityCounters {
                content: ContentUpdates::new(2),
                reactions: ReactionUpdates::new(1),
                mentions: MentionUpdates::new(2),
                active: Activity::Active,
                ..ActivityCounters::default()
            }
        );
    }

    #[test]
    fn an_update_with_no_summary_marks_activity_and_counts_nothing() {
        let mut counters = ActivityCounters::default();
        counters.advance(None, Some(user(1)));
        assert_eq!(
            counters,
            ActivityCounters {
                active: Activity::Active,
                ..ActivityCounters::default()
            }
        );
    }
}
