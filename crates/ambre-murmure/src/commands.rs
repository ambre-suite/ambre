//! Murmure's command registry: one `Command` per shortcut the shell routes
//! (the Murmure keyboard spec, `UI-THREAD-019`).
//!
//! The chords that act inside a conversation, such as the up arrow of an empty
//! composer, land with the message list. The find chord opens the search of
//! Murmure, as it opens the find of the other applications.
//!
//! The actions of a message are commands of the same table, with no chord
//! (`MURMURE-419`). The menu of a message offers the ones its target allows.

use ambre_bus::{ChannelId, LogicCommand, MessageAt, MessageId, MurmureCommand};
use ambre_commands::{
    Command, CommandCategory, CommandId, Key, KeyBinding, Keymap, Locale, Modifiers,
};

use crate::{MessageRef, MurmurePane};

/// Commands that belong to the application shell rather than a channel.
const GENERAL: CommandCategory = CommandCategory("general");
/// Moving from one channel to another.
const NAVIGATE: CommandCategory = CommandCategory("navigate");
/// The read state of the channels.
const UNREAD: CommandCategory = CommandCategory("unread");
/// Finding a message by its words.
const SEARCH_CATEGORY: CommandCategory = CommandCategory("search");
/// The actions of one message, offered in its floating menu.
const MESSAGE: CommandCategory = CommandCategory("message");

/// Open the command palette, channels first: a UI-only intent the shell
/// intercepts (`CHROME-008`).
pub const PALETTE: CommandId = CommandId("murmure.palette.open");
/// Open the next channel with activity, in sidebar order.
pub const NEXT_ACTIVITY: CommandId = CommandId("murmure.navigate.next_activity");
/// Open the previous channel with activity, in sidebar order.
pub const PREVIOUS_ACTIVITY: CommandId = CommandId("murmure.navigate.previous_activity");
/// Mark every channel of the user read (the Murmure unread spec).
pub const MARK_ALL_READ: CommandId = CommandId("murmure.unread.mark_all_read");
/// Open the search over the loaded segments, on the find chord of every
/// application (the Murmure search spec).
pub const SEARCH: CommandId = CommandId("murmure.search.open");
/// Open the thread of the message the actions are open on.
pub const REPLY: CommandId = CommandId("murmure.message.reply");
/// Show the channel unread from the message on (the Murmure unread spec).
pub const MARK_UNREAD: CommandId = CommandId("murmure.message.mark_unread");
/// Put the text of the message on the clipboard.
pub const COPY_TEXT: CommandId = CommandId("murmure.message.copy_text");

/// Alt and Shift together, the modifiers of the jumps.
const ALT_SHIFT: Modifiers = Modifiers {
    primary: false,
    alt: true,
    shift: true,
};

fn palette_title(_: &Locale) -> String {
    "Command palette".into()
}

fn next_activity_title(_: &Locale) -> String {
    "Next channel with activity".into()
}

fn previous_activity_title(_: &Locale) -> String {
    "Previous channel with activity".into()
}

fn mark_all_read_title(_: &Locale) -> String {
    "Mark everything read".into()
}

fn search_title(_: &Locale) -> String {
    "Search messages".into()
}

fn reply_title(_: &Locale) -> String {
    "Reply in thread".into()
}

fn mark_unread_title(_: &Locale) -> String {
    "Mark unread".into()
}

fn copy_text_title(_: &Locale) -> String {
    "Copy text".into()
}

/// Every command Murmure contributes, described once.
pub const COMMANDS: &[Command] = &[
    Command {
        id: PALETTE,
        title: palette_title,
        icon: None,
        keybindings: &[KeyBinding::primary('k')],
        category: GENERAL,
    },
    Command {
        id: NEXT_ACTIVITY,
        title: next_activity_title,
        icon: None,
        keybindings: &[KeyBinding::new(ALT_SHIFT, Key::ArrowDown)],
        category: NAVIGATE,
    },
    Command {
        id: PREVIOUS_ACTIVITY,
        title: previous_activity_title,
        icon: None,
        keybindings: &[KeyBinding::new(ALT_SHIFT, Key::ArrowUp)],
        category: NAVIGATE,
    },
    Command {
        id: MARK_ALL_READ,
        title: mark_all_read_title,
        icon: None,
        // Not Shift+Escape: Chrome opens its task manager on it, and the
        // page never sees the key.
        keybindings: &[KeyBinding::new(ALT_SHIFT, Key::Char('a'))],
        category: UNREAD,
    },
    Command {
        id: SEARCH,
        title: search_title,
        icon: None,
        keybindings: &[KeyBinding::primary('f')],
        category: SEARCH_CATEGORY,
    },
    // The actions of a message take no chord: a chord needs a message to
    // act on, and the keyboard reaches them through the focused one.
    Command {
        id: REPLY,
        title: reply_title,
        icon: None,
        keybindings: &[],
        category: MESSAGE,
    },
    Command {
        id: MARK_UNREAD,
        title: mark_unread_title,
        icon: None,
        keybindings: &[],
        category: MESSAGE,
    },
    Command {
        id: COPY_TEXT,
        title: copy_text_title,
        icon: None,
        keybindings: &[],
        category: MESSAGE,
    },
];

/// The message the actions are open on, as the view found it under the
/// pointer (the Murmure message list spec).
#[derive(Debug, Clone, PartialEq)]
pub struct MessageTarget {
    /// The channel the message lies in.
    pub channel: ChannelId,
    /// Where it lies: a segment of the channel, or the thread of a root.
    pub at: MessageAt,
    /// The message.
    pub message: MessageId,
    /// Its text, empty for a deleted message or a notice.
    pub text: String,
    /// The message is a notice the channel wrote, not a member.
    pub notice: bool,
    /// Where a long press held, in the pixels of the region's canvas. `None`
    /// for a right-click, whose event places the menu itself.
    pub pressed: Option<(f32, f32)>,
    /// The id of the canvas the region paints.
    pub canvas: &'static str,
}

/// What an action of a message asks the page to do.
#[derive(Debug, Clone)]
pub enum MessageAction {
    /// Show this pane.
    Open(MurmurePane),
    /// Send this command to the Logic thread.
    Send(LogicCommand),
    /// Put this text on the clipboard.
    Copy(String),
}

/// The actions offered on `target`, in menu order. A notice takes no reply,
/// a reply takes no thread of its own, and a message with no text no copy.
pub fn message_actions(target: &MessageTarget) -> Vec<CommandId> {
    let in_channel = matches!(target.at, MessageAt::Segment(_));
    let mut ids = Vec::with_capacity(3);
    if in_channel && !target.notice {
        ids.push(REPLY);
    }
    if in_channel {
        ids.push(MARK_UNREAD);
    }
    if !target.text.is_empty() {
        ids.push(COPY_TEXT);
    }
    ids
}

/// What the action `id` does on `target`. `None` for an id it does not
/// offer there.
pub fn message_action(id: CommandId, target: &MessageTarget) -> Option<MessageAction> {
    if !message_actions(target).contains(&id) {
        return None;
    }
    let segment = match target.at {
        MessageAt::Segment(segment) => Some(segment),
        MessageAt::Thread(_) => None,
    };
    match id {
        REPLY => Some(MessageAction::Open(MurmurePane::Thread {
            channel: target.channel,
            root: MessageRef {
                segment: segment?,
                message: target.message,
            },
        })),
        MARK_UNREAD => Some(MessageAction::Send(LogicCommand::Channel {
            channel_id: target.channel,
            command: MurmureCommand::MarkUnread {
                segment: segment?,
                message: target.message,
            },
        })),
        COPY_TEXT => Some(MessageAction::Copy(target.text.clone())),
        _ => None,
    }
}

/// Bind every chord of the table: the shell routes each one.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind_table(COMMANDS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::SegmentId;
    use ambre_commands::shared;

    fn target(at: MessageAt, text: &str, notice: bool) -> MessageTarget {
        MessageTarget {
            channel: ChannelId::new(),
            at,
            message: MessageId::new(),
            text: text.to_owned(),
            notice,
            pressed: None,
            canvas: "ambre-canvas",
        }
    }

    /// Every action of a message is a command of the table, so the menu,
    /// the palette and a chord reach the same entry (`MURMURE-419`).
    #[test]
    fn every_action_of_a_message_is_a_command_of_the_table() {
        let full = target(MessageAt::Segment(SegmentId::new()), "hi", false);
        let ids = message_actions(&full);
        assert_eq!(ids, [REPLY, MARK_UNREAD, COPY_TEXT]);
        for id in ids {
            assert!(ambre_commands::find(COMMANDS, id).is_some(), "{}", id.0);
        }
    }

    /// A notice takes no reply, a reply in a thread neither a thread nor a
    /// cursor, and a message with no text no copy (`MURMURE-419`).
    #[test]
    fn the_actions_follow_the_message() {
        let segment = MessageAt::Segment(SegmentId::new());
        let notice = target(segment, "joined", true);
        assert_eq!(message_actions(&notice), [MARK_UNREAD, COPY_TEXT]);
        let reply = target(MessageAt::Thread(MessageId::new()), "one", false);
        assert_eq!(message_actions(&reply), [COPY_TEXT]);
        let deleted = target(segment, "", false);
        assert_eq!(message_actions(&deleted), [REPLY, MARK_UNREAD]);
        assert!(message_action(REPLY, &reply).is_none(), "not offered");
        assert!(message_action(SEARCH, &deleted).is_none(), "not an action");
    }

    /// Reply opens the thread of the message, mark unread sends the cursor
    /// back to it, and copy hands its text over (`MURMURE-419`).
    #[test]
    fn each_action_does_what_its_title_says() {
        let segment = SegmentId::new();
        let target = target(MessageAt::Segment(segment), "hi", false);
        let root = MessageRef {
            segment,
            message: target.message,
        };
        let Some(MessageAction::Open(MurmurePane::Thread { channel, root: at })) =
            message_action(REPLY, &target)
        else {
            panic!("reply opens a thread");
        };
        assert_eq!((channel, at), (target.channel, root));
        let Some(MessageAction::Send(LogicCommand::Channel {
            channel_id,
            command:
                MurmureCommand::MarkUnread {
                    segment: s,
                    message,
                },
        })) = message_action(MARK_UNREAD, &target)
        else {
            panic!("mark unread sends the cursor back");
        };
        assert_eq!(
            (channel_id, s, message),
            (target.channel, segment, target.message)
        );
        let Some(MessageAction::Copy(text)) = message_action(COPY_TEXT, &target) else {
            panic!("copy hands the text over");
        };
        assert_eq!(text, "hi");
    }

    /// Each command is described once, with a title in every locale, a
    /// group, no blank icon and no chord another command holds.
    /// Every shortcut is therefore one command of the table (`MURMURE-170`).
    #[test]
    fn the_table_has_no_defect() {
        assert_eq!(ambre_commands::table_defects(COMMANDS), []);
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        let bound = COMMANDS.iter().flat_map(|c| c.keybindings).count();
        assert_eq!(keymap.len(), bound);
    }

    /// No chord of the table is one the browser services. Those are the
    /// clipboard chords, modern and legacy, and the undo and redo chords.
    /// Nor is it one the browser keeps from the page, such as the tab chords
    /// or the task manager of Chrome (`MURMURE-171`).
    #[test]
    fn no_chord_is_one_the_browser_services() {
        let reserved = [
            KeyBinding::shift(Key::Escape),
            KeyBinding::primary('w'),
            KeyBinding::primary('t'),
            KeyBinding::primary('n'),
            KeyBinding::primary_shift('w'),
            KeyBinding::primary_shift('t'),
            KeyBinding::primary_shift('n'),
            KeyBinding::new(Modifiers::PRIMARY, Key::Tab),
        ];
        let serviced: Vec<KeyBinding> = [
            shared::CUT_CHORDS,
            shared::COPY_CHORDS,
            shared::PASTE_CHORDS,
            shared::PASTE_PLAIN_CHORDS,
            shared::UNDO_CHORDS,
            shared::REDO_CHORDS,
            &reserved,
        ]
        .concat();
        for command in COMMANDS {
            for chord in command.keybindings {
                assert!(!serviced.contains(chord), "{} takes {chord}", command.id.0);
            }
        }
    }
}
