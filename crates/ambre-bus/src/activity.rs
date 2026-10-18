//! The activity summary an application may add to an update
//! (`ambre-spec-residency.md section 3`).
//!
//! It tells a client what an update holds with no document loaded: the kind of
//! change, and who it names. It travels on
//! [`SyncCommand::SendUpdate`](crate::SyncCommand::SendUpdate), so it crosses
//! the postcard hop and derives serde.

use ambre_core::activity as sealed;
use serde::{Deserialize, Serialize};

use crate::UserId;

/// What one update holds, readable with no Loro document.
///
/// A member can lie in a summary. The lie costs one false badge, and the
/// document corrects it on open (`RESID-011`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivitySummary {
    /// The kind of change the update carries.
    pub kind: ActivityKind,
    /// The users the update names.
    pub mentions: Vec<UserId>,
    /// Whether the update names every member, an `@channel`.
    pub mentions_all: bool,
    /// An application key, for example a thread root. Opaque here.
    pub anchor: Option<ActivityAnchor>,
}

/// The kinds of change a summary distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityKind {
    /// New content, a message for example.
    Content,
    /// A reaction on existing content.
    Reaction,
    /// An edit of existing content.
    Edit,
    /// Anything else, which moves no counter.
    Other,
}

/// An application key, opaque to the foundation and typed by the application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityAnchor(pub Vec<u8>);

impl ActivitySummary {
    /// The summary as the seal carries it. Past [`sealed::MAX_MENTIONS`] it
    /// names the channel instead, and an anchor past its bound is dropped,
    /// so a summary never costs its update.
    #[must_use]
    pub fn to_sealed(&self) -> sealed::ActivitySummary {
        let (mentions, mentions_all) = if self.mentions.len() > sealed::MAX_MENTIONS {
            (Vec::new(), true)
        } else {
            (self.mentions.clone(), self.mentions_all)
        };
        sealed::ActivitySummary {
            kind: match self.kind {
                ActivityKind::Content => sealed::ActivityKind::Content,
                ActivityKind::Reaction => sealed::ActivityKind::Reaction,
                ActivityKind::Edit => sealed::ActivityKind::Edit,
                ActivityKind::Other => sealed::ActivityKind::Other,
            },
            mentions,
            mentions_all,
            anchor: self
                .anchor
                .as_ref()
                .and_then(|anchor| sealed::ActivityAnchor::new(anchor.0.clone())),
        }
    }
}

impl From<sealed::ActivitySummary> for ActivitySummary {
    fn from(summary: sealed::ActivitySummary) -> Self {
        Self {
            kind: match summary.kind {
                sealed::ActivityKind::Content => ActivityKind::Content,
                sealed::ActivityKind::Reaction => ActivityKind::Reaction,
                sealed::ActivityKind::Edit => ActivityKind::Edit,
                sealed::ActivityKind::Other => ActivityKind::Other,
            },
            mentions: summary.mentions,
            mentions_all: summary.mentions_all,
            anchor: summary
                .anchor
                .map(|anchor| ActivityAnchor(anchor.as_bytes().to_vec())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A summary past the seal's bounds names the channel and drops its
    /// anchor, and one within them crosses whole both ways.
    #[test]
    fn a_summary_past_the_seals_bounds_names_the_channel() {
        let within = ActivitySummary {
            kind: ActivityKind::Edit,
            mentions: vec![UserId::from_bytes([1; 16])],
            mentions_all: false,
            anchor: Some(ActivityAnchor(vec![9; 8])),
        };
        assert_eq!(ActivitySummary::from(within.to_sealed()), within);
        let past = ActivitySummary {
            mentions: vec![UserId::from_bytes([1; 16]); sealed::MAX_MENTIONS + 1],
            anchor: Some(ActivityAnchor(vec![9; sealed::MAX_ANCHOR_LEN + 1])),
            ..within
        };
        let sealed = past.to_sealed();
        assert!(sealed.mentions.is_empty() && sealed.mentions_all);
        assert_eq!(sealed.anchor, None);
        assert!(sealed.encode().is_ok());
    }
}
