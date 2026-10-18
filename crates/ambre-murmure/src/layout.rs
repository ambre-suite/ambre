// ambre-murmure/src/layout.rs
// The view state of the Murmure layout: which pane the user is in (the Murmure
// layout spec). It is local to one window, so no peer shares it and nothing of
// it reaches the Logic thread.

use ambre_core::{ChannelId, MessageId, SegmentId};

/// A message, by the segment that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRef {
    /// The segment of the message.
    pub segment: SegmentId,
    /// The message.
    pub message: MessageId,
}

/// The pane the user is in. A narrow window shows it alone, and a wide one
/// shows it beside the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MurmurePane {
    /// The sidebar, with no channel chosen.
    #[default]
    ChannelList,
    /// A channel, in the message list.
    Channel {
        /// The channel.
        channel: ChannelId,
        /// The message the list keeps in view, or `None` for its latest.
        at: Option<MessageRef>,
    },
    /// A thread, opened from a root message of its channel.
    Thread {
        /// The channel of the root message.
        channel: ChannelId,
        /// The root message.
        root: MessageRef,
    },
}

impl MurmurePane {
    /// A channel at its latest message.
    pub fn channel(channel: ChannelId) -> Self {
        Self::Channel { channel, at: None }
    }

    /// The pane before this one. A thread goes back to its channel at the
    /// root message, a channel to the channel list (`MURMURE-168`). It is a
    /// value and sends nothing, so going back moves no read cursor
    /// (`MURMURE-169`).
    pub fn back(self) -> Self {
        match self {
            Self::ChannelList | Self::Channel { .. } => Self::ChannelList,
            Self::Thread { channel, root } => Self::Channel {
                channel,
                at: Some(root),
            },
        }
    }

    /// The channel this pane shows, if any.
    pub fn shown_channel(self) -> Option<ChannelId> {
        match self {
            Self::ChannelList => None,
            Self::Channel { channel, .. } | Self::Thread { channel, .. } => Some(channel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Back from a thread shows its channel at the root message, and back
    /// from a channel shows the channel list, which stays (`MURMURE-168`).
    #[test]
    fn back_from_a_thread_shows_its_channel_at_the_root_then_the_list() {
        let channel = ChannelId::new();
        let root = MessageRef {
            segment: SegmentId::new(),
            message: MessageId::new(),
        };
        let thread = MurmurePane::Thread { channel, root };
        assert_eq!(
            thread.back(),
            MurmurePane::Channel {
                channel,
                at: Some(root)
            }
        );
        assert_eq!(thread.back().shown_channel(), Some(channel));
        assert_eq!(thread.back().back(), MurmurePane::ChannelList);
        assert_eq!(
            MurmurePane::channel(channel).back(),
            MurmurePane::ChannelList
        );
        assert_eq!(MurmurePane::ChannelList.back(), MurmurePane::ChannelList);
        assert_eq!(MurmurePane::default().shown_channel(), None);
    }
}
