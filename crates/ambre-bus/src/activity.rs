//! The activity summary an application may add to an update
//! (`ambre-spec-residency.md section 3`).
//!
//! It tells a client what an update holds with no document loaded: the kind of
//! change, and who it names. It travels on
//! [`SyncCommand::SendUpdate`](crate::SyncCommand::SendUpdate), so it crosses
//! the postcard hop and derives serde.

use ambre_core::activity as sealed;
use serde::{Deserialize, Serialize};

use ambre_core::id::{Id, User};

pub use sealed::{Activity, ActivityAnchor, ActivityKind, CallCue, CueKind, MentionScope};

/// What one update holds, readable with no Loro document.
///
/// A member can lie in a summary. The lie costs one false badge, and the
/// document corrects it on open (`RESID-011`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivitySummary {
    /// The kind of change the update carries.
    pub kind: ActivityKind,
    /// The users the update names.
    pub mentions: Vec<Id<User>>,
    /// Whether the update names every member, an `@channel`. It writes the
    /// byte of a `bool`.
    pub mentions_all: MentionScope,
    /// An application key, for example a thread root. Opaque here.
    pub anchor: Option<ActivityAnchor>,
    /// The cue of a call, on the update that posts a call message or writes
    /// its end (the Murmure calls spec, section 3, the ring).
    pub call: Option<CallCue>,
}

impl ActivitySummary {
    /// The summary as the seal carries it. Past [`sealed::MAX_MENTIONS`] it
    /// names the channel instead, and an anchor past its bound is dropped,
    /// so a summary never costs its update.
    #[must_use]
    pub fn to_sealed(&self) -> sealed::ActivitySummary {
        let (mentions, mentions_all) = if self.mentions.len() > sealed::MAX_MENTIONS {
            (Vec::new(), MentionScope::All)
        } else {
            (self.mentions.clone(), self.mentions_all)
        };
        sealed::ActivitySummary {
            kind: self.kind,
            mentions,
            mentions_all,
            anchor: self.anchor.clone().and_then(ActivityAnchor::within_bound),
            call: self.call,
        }
    }
}

impl From<sealed::ActivitySummary> for ActivitySummary {
    fn from(summary: sealed::ActivitySummary) -> Self {
        Self {
            kind: summary.kind,
            mentions: summary.mentions,
            mentions_all: summary.mentions_all,
            anchor: summary.anchor,
            call: summary.call,
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
            mentions: vec![Id::<User>::from_bytes([1; 16])],
            mentions_all: crate::MentionScope::Named,
            anchor: ActivityAnchor::new(vec![9; 8]),
            call: None,
        };
        assert_eq!(ActivitySummary::from(within.to_sealed()), within);
        let past = ActivitySummary {
            mentions: vec![Id::<User>::from_bytes([1; 16]); sealed::MAX_MENTIONS + 1],
            anchor: Some(long_anchor()),
            ..within
        };
        let sealed = past.to_sealed();
        assert!(sealed.mentions.is_empty());
        assert_eq!(sealed.mentions_all, MentionScope::All);
        assert_eq!(sealed.anchor, None);
        assert!(sealed.encode().is_ok());
    }

    /// An anchor past the seal's bound, as the postcard hop carries it.
    fn long_anchor() -> ActivityAnchor {
        let bytes = postcard::to_allocvec(&vec![9_u8; sealed::MAX_ANCHOR_LEN + 1]).unwrap();
        postcard::from_bytes(&bytes).unwrap()
    }

    /// The postcard hop writes an anchor as its bare bytes, and reads one past
    /// the seal's bound with no refusal.
    #[test]
    fn an_anchor_crosses_the_hop_as_its_bare_bytes_whatever_its_length() {
        let anchor = ActivityAnchor::new(vec![1, 2]).unwrap();
        // A length byte, then the bytes: the layout the bus copy wrote.
        assert_eq!(postcard::to_allocvec(&anchor).unwrap(), [2, 1, 2]);
        assert_eq!(long_anchor().as_bytes().len(), sealed::MAX_ANCHOR_LEN + 1);
    }
}
