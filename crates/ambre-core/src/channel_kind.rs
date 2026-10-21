//! What a document of a Murmure channel is, and where a segment stands in its
//! life (`ambre-spec-murmure-schema.md`).
//!
//! The relay stores the kind beside each document (`ZK-015`) and the listing
//! answers it, so the client and the relay share this one closed set. A
//! segment's state is stored on both sides too: in the channel's `meta`
//! document and in the relay's register. One type spells it for both.

use serde::{Deserialize, Serialize};

/// The kind of a document a channel groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChannelDocumentKind {
    /// The one `meta` document: the channel's name, type and segment list.
    Meta,
    /// One time slice of the channel's messages.
    Segment,
    /// The corrections to a final segment.
    Amendments,
    /// The replies to one message.
    Thread,
}

impl ChannelDocumentKind {
    /// The kind a stored word names, or `None` for a word no build wrote.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "meta" => Some(Self::Meta),
            "segment" => Some(Self::Segment),
            "amendments" => Some(Self::Amendments),
            "thread" => Some(Self::Thread),
            _ => None,
        }
    }

    /// The word the relay stores, which [`ChannelDocumentKind::parse`] reads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Meta => "meta",
            Self::Segment => "segment",
            Self::Amendments => "amendments",
            Self::Thread => "thread",
        }
    }
}

/// Where a segment stands in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SegmentState {
    /// The segment new messages go to.
    Active,
    /// A segment being finalized, still open to late deltas.
    Closing,
    /// An immutable segment, loaded on demand only.
    Final,
}

impl SegmentState {
    /// The word a `segments` entry and the relay's register store.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Final => "final",
        }
    }

    /// The state a stored word names, or `None` for a word no build wrote.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "active" => Some(Self::Active),
            "closing" => Some(Self::Closing),
            "final" => Some(Self::Final),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChannelDocumentKind, SegmentState};

    /// Every kind reads back from the word it stores.
    #[test]
    fn each_kind_reads_back_from_its_stored_word() {
        use ChannelDocumentKind::{Amendments, Meta, Segment, Thread};
        for kind in [Meta, Segment, Amendments, Thread] {
            assert_eq!(ChannelDocumentKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ChannelDocumentKind::parse("space"), None);
    }

    /// Every state reads back from the word it stores.
    #[test]
    fn each_segment_state_reads_back_from_its_stored_word() {
        use SegmentState::{Active, Closing, Final};
        for state in [Active, Closing, Final] {
            assert_eq!(SegmentState::parse(state.as_str()), Some(state));
        }
        assert_eq!(SegmentState::parse("open"), None);
    }
}
