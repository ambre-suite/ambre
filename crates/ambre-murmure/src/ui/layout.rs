// ambre-murmure/src/ui/layout.rs
// The view state of the Murmure layout: which pane the user is in (the Murmure
// layout spec). It is local to one window, so no peer shares it and nothing of
// it reaches the Logic thread.

use ambre_core::id::{Channel, Id, Message, Segment};

/// A message, by the segment that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRef {
    /// The segment of the message.
    pub segment: Id<Segment>,
    /// The message.
    pub message: Id<Message>,
}

/// A region of the page that shows a list: the channel in the content
/// canvas, or the thread in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The channel the pane shows, in the content canvas.
    Channel,
    /// The thread the pane shows, in the panel.
    Thread,
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
        channel: Id<Channel>,
        /// The message the list keeps in view, or `None` for its latest.
        at: Option<MessageRef>,
    },
    /// A thread, opened from a root message of its channel.
    Thread {
        /// The channel of the root message.
        channel: Id<Channel>,
        /// The root message.
        root: MessageRef,
    },
    /// The member list of the space a channel lies in, beside that channel
    /// (the Murmure members spec).
    Members {
        /// The channel the list was opened from.
        channel: Id<Channel>,
    },
}

impl MurmurePane {
    /// A channel at its latest message.
    pub fn channel(channel: Id<Channel>) -> Self {
        Self::Channel { channel, at: None }
    }

    /// The pane before this one. A thread goes back to its channel at the
    /// root message, a channel to the channel list (`MURMURE-168`). The
    /// member list goes back to its channel. It is a value and sends nothing,
    /// so going back moves no read cursor (`MURMURE-169`).
    pub fn back(self) -> Self {
        match self {
            Self::ChannelList | Self::Channel { .. } => Self::ChannelList,
            Self::Thread { channel, root } => Self::Channel {
                channel,
                at: Some(root),
            },
            Self::Members { channel } => Self::channel(channel),
        }
    }

    /// The channel this pane shows, if any.
    pub fn shown_channel(self) -> Option<Id<Channel>> {
        match self {
            Self::ChannelList => None,
            Self::Channel { channel, .. }
            | Self::Thread { channel, .. }
            | Self::Members { channel } => Some(channel),
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
        let channel = Id::<Channel>::mint();
        let root = MessageRef {
            segment: Id::<Segment>::mint(),
            message: Id::<Message>::mint(),
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

    /// The member list shows beside its channel, and back from it shows that
    /// channel at its latest message.
    #[test]
    fn back_from_the_member_list_shows_its_channel() {
        let channel = Id::<Channel>::mint();
        let members = MurmurePane::Members { channel };
        assert_eq!(members.shown_channel(), Some(channel));
        assert_eq!(members.back(), MurmurePane::channel(channel));
    }
}
