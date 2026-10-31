//! Murmure's command registry: one `Command` per shortcut the shell routes
//! (the Murmure keyboard spec, `UI-THREAD-019`).
//!
//! A key inside a conversation, such as the up arrow of an empty composer, is
//! the message region's. The region holds the focus it needs. The find chord opens the search of Murmure, as it opens the find of the
//! other applications.
//!
//! The actions of a message are commands of the same table, with no chord
//! (`MURMURE-419`). The menu of a message offers the ones its target allows.

use ambre_bus::{
    Emoji, HeldReply, HeldSend, LogicCommand, Mention, MessageAt, MurmureCommand, StyleMark,
    Timestamp,
};
use ambre_commands::{
    Command, CommandCategory, CommandId, FunctionKey, Key, KeyBinding, KeyChar, Keymap, Locale,
    Modifiers,
};
use ambre_core::ByteCount;
use ambre_core::id::{Channel, Document, Id, Message, User};

use crate::{MessageRef, MurmurePane, Region};

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
/// Load a message of the member into the composer, to send it edited
/// (`MURMURE-203`).
pub const EDIT: CommandId = CommandId("murmure.message.edit");
/// Delete a message of the member, once a dialog confirms it (`MURMURE-512`).
pub const DELETE: CommandId = CommandId("murmure.message.delete");
/// Open the emoji picker on the message, to react with the emoji picked
/// (`MURMURE-140`).
pub const REACT: CommandId = CommandId("murmure.message.react");
/// Open the list of the keys Murmure takes (`MURMURE-540`).
pub const KEYS: CommandId = CommandId("murmure.general.keys");

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

fn react_title(_: &Locale) -> String {
    "Add reaction".into()
}

fn copy_text_title(_: &Locale) -> String {
    "Copy text".into()
}

fn edit_title(_: &Locale) -> String {
    "Edit message".into()
}

fn delete_title(_: &Locale) -> String {
    "Delete message".into()
}

fn keys_title(_: &Locale) -> String {
    "Keyboard shortcuts".into()
}

/// Every command Murmure contributes, described once.
pub const COMMANDS: &[Command] = &[
    Command {
        id: PALETTE,
        title: palette_title,
        icon: None,
        keybindings: &[KeyBinding::primary(KeyChar::K)],
        category: GENERAL,
    },
    Command {
        id: NEXT_ACTIVITY,
        title: next_activity_title,
        icon: None,
        keybindings: &[KeyBinding::new(Modifiers::ALT_SHIFT, Key::ArrowDown)],
        category: NAVIGATE,
    },
    Command {
        id: PREVIOUS_ACTIVITY,
        title: previous_activity_title,
        icon: None,
        keybindings: &[KeyBinding::new(Modifiers::ALT_SHIFT, Key::ArrowUp)],
        category: NAVIGATE,
    },
    Command {
        id: MARK_ALL_READ,
        title: mark_all_read_title,
        icon: None,
        // Not Shift+Escape: Chrome opens its task manager on it, and the
        // page never sees the key.
        keybindings: &[KeyBinding::new(Modifiers::ALT_SHIFT, Key::Char(KeyChar::A))],
        category: UNREAD,
    },
    Command {
        id: SEARCH,
        title: search_title,
        icon: None,
        keybindings: &[KeyBinding::primary(KeyChar::F)],
        category: SEARCH_CATEGORY,
    },
    // No chord: the keymap matches the character a key types, and Ctrl+/
    // needs Shift on a French keyboard. The palette reaches it.
    Command {
        id: KEYS,
        title: keys_title,
        icon: None,
        keybindings: &[],
        category: GENERAL,
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
        id: REACT,
        title: react_title,
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
    Command {
        id: EDIT,
        title: edit_title,
        icon: None,
        keybindings: &[],
        category: MESSAGE,
    },
    Command {
        id: DELETE,
        title: delete_title,
        icon: None,
        keybindings: &[],
        category: MESSAGE,
    },
];

ambre_core::define_flag! {
    /// Who wrote a message the actions are open on.
    pub enum Writer {
        /// A member wrote it.
        Member,
        /// The channel wrote it: a notice, which takes no reply and no
        /// reaction.
        Channel,
    }
}

/// The message the actions are open on, as the view found it under the
/// pointer (the Murmure message list spec).
///
/// `Debug` prints the length of the text alone: it is plaintext a member
/// wrote.
#[derive(Clone, PartialEq)]
pub struct MessageTarget {
    /// The channel the message lies in.
    pub channel: Id<Channel>,
    /// Where it lies: a segment of the channel, or the thread of a root.
    pub at: MessageAt,
    /// The message.
    pub message: Id<Message>,
    /// The member who wrote it.
    pub author: Id<User>,
    /// Its text, empty for a deleted message or a notice.
    pub text: String,
    /// Who its text names, as the list holds them.
    pub mentions: Vec<Mention>,
    /// How its runs read, as the list holds them.
    pub styles: Vec<StyleMark>,
    /// Who wrote the message: a member, or the channel as a notice.
    pub writer: Writer,
    /// Whether the message reads deleted already.
    pub deleted: ambre_bus::Deletion,
    /// Where a long press held, in the pixels of the region's canvas. `None`
    /// for a right-click, whose event places the menu itself.
    pub pressed: Option<ambre_core::Point<ambre_core::Px<ambre_core::View>>>,
    /// The id of the canvas the region paints.
    pub canvas: ambre_core::CanvasId,
    /// The thread a reply also sent to the channel came from, `None` on any
    /// other message (`MURMURE-533`).
    pub from_thread: Option<FromThread>,
}

impl std::fmt::Debug for MessageTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageTarget")
            .field("channel", &self.channel)
            .field("at", &self.at)
            .field("message", &self.message)
            .field("author", &self.author)
            .field("text", &ByteCount::from_len(self.text.len()))
            .field("mentions", &self.mentions.len())
            .field("styles", &self.styles.len())
            .field("writer", &self.writer)
            .field("deleted", &self.deleted)
            .field("pressed", &self.pressed)
            .field("canvas", &self.canvas)
            .field("from_thread", &self.from_thread)
            .finish()
    }
}

/// Where the root of the thread a copy came from lies, as the list holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FromThread {
    /// In a segment the list holds.
    Root(MessageRef),
    /// In a segment the list has not loaded. The copy names its thread
    /// document, whose header names the segment to load down to.
    Unloaded(Id<Document>),
}

/// What an action of a message asks the page to do.
///
/// `Debug` prints the length of the text a copy carries alone.
#[derive(Clone)]
pub enum MessageAction {
    /// Show this pane.
    Open(MurmurePane),
    /// Send this command to the Logic thread.
    Send(LogicCommand),
    /// Put this text on the clipboard.
    Copy(String),
    /// Ask the member to confirm this deletion before it is sent.
    ConfirmDelete(PendingDelete),
    /// Load this message into the composer of its channel or its thread.
    Edit(PendingEdit),
    /// Open the emoji picker, whose pick reacts to this message.
    React(PendingReaction),
}

impl std::fmt::Debug for MessageAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(pane) => f.debug_tuple("Open").field(pane).finish(),
            Self::Send(command) => f.debug_tuple("Send").field(command).finish(),
            Self::Copy(text) => f
                .debug_tuple("Copy")
                .field(&ByteCount::from_len(text.len()))
                .finish(),
            Self::ConfirmDelete(delete) => f.debug_tuple("ConfirmDelete").field(delete).finish(),
            Self::Edit(edit) => f.debug_tuple("Edit").field(edit).finish(),
            Self::React(reaction) => f.debug_tuple("React").field(reaction).finish(),
        }
    }
}

/// A message the emoji picker reacts to, once an emoji is picked
/// (`MURMURE-140`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingReaction {
    /// The channel the message lies in.
    pub channel: Id<Channel>,
    /// Where it lies: a segment of the channel, or the thread of a root.
    pub at: MessageAt,
    /// The message.
    pub message: Id<Message>,
}

impl PendingReaction {
    /// The command that reacts with `emoji`, stamped `now` by the view
    /// (`MURMURE-092`).
    pub fn command(self, emoji: Emoji, now: Timestamp) -> LogicCommand {
        LogicCommand::Channel {
            channel_id: self.channel,
            command: MurmureCommand::React {
                at: self.at,
                message: self.message,
                emoji,
                now,
            },
        }
    }
}

/// What the emoji picker is open for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picking {
    /// The composer of a region, which inserts the pick at its caret
    /// (`MURMURE-139`).
    Composer { region: Region },
    /// A message, which the pick reacts to (`MURMURE-140`).
    Reaction(PendingReaction),
}

/// A message the member asked to edit, which the composer of its channel
/// loads (`MURMURE-203`).
///
/// `Debug` prints the length of the text alone.
#[derive(Clone, PartialEq, Eq)]
pub struct PendingEdit {
    /// The channel the message lies in.
    pub channel: Id<Channel>,
    /// Where it lies. A reply loads into the composer of its thread.
    pub at: MessageAt,
    /// The message.
    pub message: Id<Message>,
    /// Its text as the list holds it.
    pub text: String,
    /// Who that text names.
    pub mentions: Vec<Mention>,
    /// How its runs read.
    pub styles: Vec<StyleMark>,
}

impl std::fmt::Debug for PendingEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingEdit")
            .field("channel", &self.channel)
            .field("at", &self.at)
            .field("message", &self.message)
            .field("text", &ByteCount::from_len(self.text.len()))
            .field("mentions", &self.mentions.len())
            .field("styles", &self.styles.len())
            .finish()
    }
}

/// A deletion the member asked for and has not confirmed yet
/// (`MURMURE-512`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingDelete {
    /// The channel the message lies in.
    pub channel: Id<Channel>,
    /// Where it lies: a segment of the channel, or the thread of a root.
    pub at: MessageAt,
    /// The message.
    pub message: Id<Message>,
}

impl PendingDelete {
    /// The command that deletes the message, stamped `now` by the view
    /// (`MURMURE-087`).
    pub fn command(self, now: Timestamp) -> LogicCommand {
        LogicCommand::Channel {
            channel_id: self.channel,
            command: MurmureCommand::DeleteMessage {
                at: self.at,
                message: self.message,
                now,
            },
        }
    }
}

/// An `@channel` the Logic thread holds until its author answers the
/// question (`MURMURE-557`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskedAll {
    /// The channel the send writes in.
    pub channel: Id<Channel>,
    /// How many members the channel holds, as the question says.
    pub members: ambre_core::MemberCount,
    /// The draft, and the thread it answers for a reply.
    pub held: HeldSend,
}

impl AskedAll {
    /// The send again, confirmed and stamped `now` by the view
    /// (`MURMURE-557`).
    pub fn command(self, now: Timestamp) -> LogicCommand {
        let HeldSend {
            draft,
            attachments,
            reply,
        } = self.held;
        let command = match reply {
            None => MurmureCommand::SendMessage {
                draft,
                attachments,
                now,
                confirmed: ambre_bus::Confirmation::Confirmed,
            },
            Some(HeldReply {
                root,
                also_in_channel,
            }) => MurmureCommand::Reply {
                root,
                draft,
                also_in_channel,
                now,
                confirmed: ambre_bus::Confirmation::Confirmed,
            },
        };
        LogicCommand::Channel {
            channel_id: self.channel,
            command,
        }
    }

    /// The region that takes the send back: the thread's for a reply.
    pub fn region(&self) -> Region {
        if self.held.reply.is_some() {
            Region::Thread
        } else {
            Region::Channel
        }
    }
}

/// The question an `@channel` past the member threshold asks.
pub fn asked_all_title(members: ambre_core::MemberCount) -> String {
    format!("Notify all {} members of this channel?", members.get())
}

/// The actions offered on `target` to the member `me`, in menu order. A
/// notice takes no reply, a reply takes no thread of its own, and a message
/// with no text no copy. Only the author deletes or edits a message. A reply
/// is edited in the composer of its thread (`MURMURE-526`).
pub fn message_actions(target: &MessageTarget, me: Option<Id<User>>) -> Vec<CommandId> {
    let in_channel = matches!(target.at, MessageAt::Segment(_));
    let by_member = target.writer == Writer::Member;
    let mut ids = Vec::with_capacity(6);
    if in_channel && by_member {
        ids.push(REPLY);
    }
    if by_member && target.deleted == ambre_bus::Deletion::Live {
        ids.push(REACT);
    }
    if in_channel {
        ids.push(MARK_UNREAD);
    }
    if !target.text.is_empty() {
        ids.push(COPY_TEXT);
    }
    let own = me == Some(target.author) && by_member && target.deleted == ambre_bus::Deletion::Live;
    if own {
        ids.extend([EDIT, DELETE]);
    }
    ids
}

/// What the action `id` does on `target` for the member `me`. `None` for an
/// id it does not offer there.
pub fn message_action(
    id: CommandId,
    target: &MessageTarget,
    me: Option<Id<User>>,
) -> Option<MessageAction> {
    if !message_actions(target, me).contains(&id) {
        return None;
    }
    let segment = match target.at {
        MessageAt::Segment(segment) => Some(segment),
        MessageAt::Thread(_) => None,
    };
    match id {
        // A copy opens the thread it came from (`MURMURE-533`). Its root
        // unloaded, Logic loads the segments down to it first, then answers
        // with the root (`MURMURE-545`).
        REPLY => match target.from_thread {
            Some(FromThread::Unloaded(thread)) => {
                Some(MessageAction::Send(LogicCommand::Channel {
                    channel_id: target.channel,
                    command: MurmureCommand::SeekRoot { thread },
                }))
            }
            Some(FromThread::Root(root)) => Some(MessageAction::Open(MurmurePane::Thread {
                channel: target.channel,
                root,
            })),
            None => Some(MessageAction::Open(MurmurePane::Thread {
                channel: target.channel,
                root: MessageRef {
                    segment: segment?,
                    message: target.message,
                },
            })),
        },
        MARK_UNREAD => Some(MessageAction::Send(LogicCommand::Channel {
            channel_id: target.channel,
            command: MurmureCommand::MarkUnread {
                segment: segment?,
                message: target.message,
            },
        })),
        REACT => Some(MessageAction::React(PendingReaction {
            channel: target.channel,
            at: target.at,
            message: target.message,
        })),
        COPY_TEXT => Some(MessageAction::Copy(target.text.clone())),
        DELETE => Some(MessageAction::ConfirmDelete(PendingDelete {
            channel: target.channel,
            at: target.at,
            message: target.message,
        })),
        EDIT => Some(MessageAction::Edit(PendingEdit {
            channel: target.channel,
            at: target.at,
            message: target.message,
            text: target.text.clone(),
            mentions: target.mentions.clone(),
            styles: target.styles.clone(),
        })),
        _ => None,
    }
}

/// Bind every chord of the table: the shell routes each one.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind_table(COMMANDS);
}

/// Where a key of the list acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPlace {
    /// Anywhere in Murmure: a chord of the command table.
    Anywhere,
    /// The message list of a channel.
    List,
    /// The message list of a thread.
    ThreadList,
    /// The composer of a channel or of a thread.
    Composer,
    /// The composer of a thread alone.
    ThreadComposer,
}

impl KeyPlace {
    /// The words the list shows for the place.
    pub fn words(self) -> &'static str {
        match self {
            Self::Anywhere => "Anywhere",
            Self::List => "The message list",
            Self::ThreadList => "The list of a thread",
            Self::Composer => "The composer",
            Self::ThreadComposer => "The composer of a thread",
        }
    }
}

/// One key a region takes, which the shell never sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionKey {
    /// The key and its modifiers.
    pub keys: KeyBinding,
    /// What it does.
    pub action: &'static str,
    /// Where it acts.
    pub place: KeyPlace,
}

const fn region_key(keys: KeyBinding, action: &'static str, place: KeyPlace) -> RegionKey {
    RegionKey {
        keys,
        action,
        place,
    }
}

/// The key that, with the primary modifier, opens the emoji picker on the
/// composer (`MURMURE-139`).
pub const EMOJI_KEY: KeyBinding = KeyBinding::primary(KeyChar::new(';'));

/// The keys inside a conversation, one row each (the Murmure keyboard spec).
pub const REGION_KEYS: &[RegionKey] = &[
    region_key(
        KeyBinding::shift(Key::PageUp),
        "Jump to the first unread message",
        KeyPlace::List,
    ),
    region_key(
        KeyBinding::new(Modifiers::NONE, Key::Escape),
        "Mark the channel read",
        KeyPlace::List,
    ),
    region_key(
        KeyBinding::new(Modifiers::NONE, Key::Escape),
        "Close the thread",
        KeyPlace::ThreadList,
    ),
    region_key(
        KeyBinding::shift(Key::F(FunctionKey::F10)),
        "Open the actions of the focused message",
        KeyPlace::List,
    ),
    region_key(
        KeyBinding::new(Modifiers::NONE, Key::Enter),
        "Send",
        KeyPlace::Composer,
    ),
    region_key(
        KeyBinding::shift(Key::Enter),
        "Break the line",
        KeyPlace::Composer,
    ),
    region_key(
        KeyBinding::new(Modifiers::NONE, Key::ArrowUp),
        "Edit your last message, from an empty composer",
        KeyPlace::Composer,
    ),
    region_key(
        KeyBinding::new(Modifiers::NONE, Key::Escape),
        "Cancel the edit",
        KeyPlace::Composer,
    ),
    // Unshifted on a QWERTY and on a French keyboard alike, and a chord no
    // browser services.
    region_key(EMOJI_KEY, "Insert an emoji", KeyPlace::Composer),
    region_key(
        KeyBinding::new(Modifiers::ALT, Key::Enter),
        "Also send the reply to the channel",
        KeyPlace::ThreadComposer,
    ),
];

/// One row of the list of keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    /// The key and its modifiers.
    pub keys: KeyBinding,
    /// What it does.
    pub action: String,
    /// Where it acts.
    pub place: KeyPlace,
}

/// The list of keys: each chord of the command table, then each key of a
/// conversation (`MURMURE-540`). Both come from their one table.
pub fn key_list(locale: &Locale) -> Vec<KeyRow> {
    let chords = COMMANDS.iter().flat_map(|command| {
        command.keybindings.iter().map(|keys| KeyRow {
            keys: *keys,
            action: (command.title)(locale),
            place: KeyPlace::Anywhere,
        })
    });
    let region = REGION_KEYS.iter().map(|key| KeyRow {
        keys: key.keys,
        action: key.action.to_owned(),
        place: key.place,
    });
    chords.chain(region).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::ReplyEcho;
    use ambre_commands::shared;
    use ambre_core::id::Segment;

    fn target(at: MessageAt, text: &str, notice: bool) -> MessageTarget {
        MessageTarget {
            channel: Id::<Channel>::mint(),
            at,
            message: Id::<Message>::mint(),
            author: Id::<User>::mint(),
            text: text.to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
            writer: Writer::from_bool(notice),
            deleted: ambre_bus::Deletion::from_bool(text.is_empty()),
            pressed: None,
            canvas: ambre_core::CanvasId::CONTENT,
            from_thread: None,
        }
    }

    /// A target, an edit and a copy each debug the length of the text alone.
    /// A message is plaintext.
    #[test]
    fn a_message_text_debugs_its_length_alone() {
        let target = target(
            MessageAt::Segment(Id::<Segment>::mint()),
            "secret words",
            false,
        );
        let edit = PendingEdit {
            channel: target.channel,
            at: target.at,
            message: target.message,
            text: target.text.clone(),
            mentions: Vec::new(),
            styles: Vec::new(),
        };
        let send = MessageAction::Send(LogicCommand::Channel {
            channel_id: target.channel,
            command: MurmureCommand::EditMessage {
                at: target.at,
                message: target.message,
                draft: ambre_bus::MessageDraft {
                    text: target.text.clone(),
                    mentions: Vec::new(),
                    styles: Vec::new(),
                },
                now: Timestamp::default(),
            },
        });
        let shown = [
            (format!("{target:?}"), "text: ByteCount(12)"),
            (format!("{edit:?}"), "text: ByteCount(12)"),
            (
                format!("{:?}", MessageAction::Copy(target.text.clone())),
                "Copy(ByteCount(12))",
            ),
            (format!("{send:?}"), "text: 12 chars"),
        ];
        for (shown, length) in shown {
            assert!(!shown.contains("secret"), "{shown}");
            assert!(shown.contains(length), "{shown}");
        }
    }

    /// The answer to the `@channel` question sends the held draft again,
    /// confirmed, to the channel or the thread it was for (`MURMURE-557`).
    #[test]
    fn a_confirmed_at_channel_sends_the_held_draft_again() {
        let channel = Id::<Channel>::mint();
        let draft = ambre_bus::MessageDraft {
            text: "@channel hi".to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
        };
        let asked = AskedAll {
            channel,
            members: ambre_core::MemberCount::new(12),
            held: HeldSend {
                draft: draft.clone(),
                attachments: Vec::new(),
                reply: None,
            },
        };
        assert_eq!(asked.region(), Region::Channel);
        assert!(matches!(
            asked.clone().command(Timestamp(7)),
            LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::SendMessage { draft: d, now: Timestamp(7), confirmed: ambre_bus::Confirmation::Confirmed, .. },
            } if channel_id == channel && d == draft
        ));
        let root = Id::<Message>::mint();
        let reply = AskedAll {
            held: HeldSend {
                draft: draft.clone(),
                attachments: Vec::new(),
                reply: Some(HeldReply {
                    root,
                    also_in_channel: ReplyEcho::AlsoInChannel,
                }),
            },
            ..asked
        };
        assert_eq!(reply.region(), Region::Thread);
        assert!(matches!(
            reply.command(Timestamp(8)),
            LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::Reply {
                    root: r,
                    draft: d,
                    also_in_channel: ReplyEcho::AlsoInChannel,
                    now: Timestamp(8),
                    confirmed: ambre_bus::Confirmation::Confirmed,
                },
            } if channel_id == channel && r == root && d == draft
        ));
        assert_eq!(
            asked_all_title(ambre_core::MemberCount::new(12)),
            "Notify all 12 members of this channel?"
        );
    }

    /// Every action of a message is a command of the table, so the menu,
    /// the palette and a chord reach the same entry (`MURMURE-419`).
    #[test]
    fn every_action_of_a_message_is_a_command_of_the_table() {
        let full = target(MessageAt::Segment(Id::<Segment>::mint()), "hi", false);
        let ids = message_actions(&full, Some(full.author));
        assert_eq!(ids, [REPLY, REACT, MARK_UNREAD, COPY_TEXT, EDIT, DELETE]);
        for id in ids {
            assert!(ambre_commands::find(COMMANDS, id).is_some(), "{}", id.0);
        }
    }

    /// A notice takes no reply, a reply in a thread neither a thread nor a
    /// cursor, and a message with no text no copy (`MURMURE-419`).
    #[test]
    fn the_actions_follow_the_message() {
        let segment = MessageAt::Segment(Id::<Segment>::mint());
        let notice = target(segment, "joined", true);
        assert_eq!(message_actions(&notice, None), [MARK_UNREAD, COPY_TEXT]);
        let reply = target(MessageAt::Thread(Id::<Message>::mint()), "one", false);
        assert_eq!(message_actions(&reply, None), [REACT, COPY_TEXT]);
        let deleted = target(segment, "", false);
        assert_eq!(message_actions(&deleted, None), [REPLY, MARK_UNREAD]);
        assert!(message_action(REPLY, &reply, None).is_none(), "not offered");
        let not_one = message_action(SEARCH, &deleted, None);
        assert!(not_one.is_none(), "not an action");
    }

    /// The reply action on a copy opens the thread it came from, never one
    /// of its own (`MURMURE-533`). A copy whose root is not loaded asks
    /// Logic to seek it (`MURMURE-545`).
    #[test]
    fn the_reply_on_a_copy_opens_the_thread_it_came_from() {
        let segment = MessageAt::Segment(Id::<Segment>::mint());
        let mut copy = target(segment, "ok", false);
        let root = MessageRef {
            segment: Id::<Segment>::mint(),
            message: Id::<Message>::mint(),
        };
        copy.from_thread = Some(FromThread::Root(root));
        let Some(MessageAction::Open(MurmurePane::Thread { channel, root: at })) =
            message_action(REPLY, &copy, None)
        else {
            panic!("the reply opens a thread");
        };
        assert_eq!((channel, at), (copy.channel, root));
        let thread = Id::<Document>::mint();
        copy.from_thread = Some(FromThread::Unloaded(thread));
        assert_eq!(
            message_actions(&copy, None),
            [REPLY, REACT, MARK_UNREAD, COPY_TEXT]
        );
        let Some(MessageAction::Send(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::SeekRoot { thread: sought },
        })) = message_action(REPLY, &copy, None)
        else {
            panic!("the reply seeks the root");
        };
        assert_eq!((channel_id, sought), (copy.channel, thread));
    }

    /// Only the author is offered the deletion of a message, in a channel or
    /// in a thread. A notice and a message deleted already offer none.
    /// The action asks for a confirmation, and sends nothing (`MURMURE-512`).
    #[test]
    fn only_the_author_deletes_a_message_and_only_once_confirmed() {
        let segment = MessageAt::Segment(Id::<Segment>::mint());
        let own = target(segment, "hi", false);
        let me = Some(own.author);
        assert_eq!(message_actions(&own, None).last(), Some(&COPY_TEXT));
        let other = Some(Id::<User>::mint());
        assert!(
            !message_actions(&own, other).contains(&DELETE),
            "not theirs"
        );
        let reply = target(MessageAt::Thread(Id::<Message>::mint()), "one", false);
        let reply_ids = message_actions(&reply, Some(reply.author));
        assert_eq!(reply_ids, [REACT, COPY_TEXT, EDIT, DELETE]);
        let notice = target(segment, "joined", true);
        let notice_ids = message_actions(&notice, Some(notice.author));
        assert!(!notice_ids.contains(&DELETE), "a notice");
        let gone = target(segment, "", false);
        let gone_ids = message_actions(&gone, Some(gone.author));
        assert!(!gone_ids.contains(&DELETE), "deleted already");

        let Some(MessageAction::ConfirmDelete(pending)) = message_action(DELETE, &own, me) else {
            panic!("delete asks for a confirmation");
        };
        let asked = PendingDelete {
            channel: own.channel,
            at: own.at,
            message: own.message,
        };
        assert_eq!(pending, asked);
        let now = Timestamp(1_700_000_000_000);
        let LogicCommand::Channel {
            channel_id,
            command:
                MurmureCommand::DeleteMessage {
                    at,
                    message,
                    now: t,
                },
        } = pending.command(now)
        else {
            panic!("the confirmation deletes the message");
        };
        assert_eq!(
            (channel_id, at, message, t),
            (own.channel, own.at, own.message, now)
        );
    }

    /// Only the author is offered the edit of a message, in the channel and
    /// in a thread. The action hands the message to the composer and sends
    /// nothing (`MURMURE-203`, `MURMURE-526`).
    #[test]
    fn only_the_author_edits_a_message_or_a_reply() {
        let segment = MessageAt::Segment(Id::<Segment>::mint());
        let own = target(segment, "hi", false);
        let me = Some(own.author);
        assert!(!message_actions(&own, Some(Id::<User>::mint())).contains(&EDIT));
        let reply = target(MessageAt::Thread(Id::<Message>::mint()), "one", false);
        let Some(MessageAction::Edit(of_reply)) = message_action(EDIT, &reply, Some(reply.author))
        else {
            panic!("a reply is edited in its thread");
        };
        assert_eq!((of_reply.at, of_reply.message), (reply.at, reply.message));
        let others = message_actions(&reply, Some(Id::<User>::mint()));
        assert!(!others.contains(&EDIT), "nor another member's reply");
        let notice = target(segment, "joined", true);
        assert!(!message_actions(&notice, Some(notice.author)).contains(&EDIT));
        let gone = target(segment, "", false);
        assert!(!message_actions(&gone, Some(gone.author)).contains(&EDIT));

        let Some(MessageAction::Edit(pending)) = message_action(EDIT, &own, me) else {
            panic!("edit loads the composer");
        };
        let asked = PendingEdit {
            channel: own.channel,
            at: own.at,
            message: own.message,
            text: "hi".to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
        };
        assert_eq!(pending, asked);
    }

    /// Reply opens the thread of the message, mark unread sends the cursor
    /// back to it, and copy hands its text over (`MURMURE-419`).
    #[test]
    fn each_action_does_what_its_title_says() {
        let segment = Id::<Segment>::mint();
        let target = target(MessageAt::Segment(segment), "hi", false);
        let root = MessageRef {
            segment,
            message: target.message,
        };
        let Some(MessageAction::Open(MurmurePane::Thread { channel, root: at })) =
            message_action(REPLY, &target, None)
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
        })) = message_action(MARK_UNREAD, &target, None)
        else {
            panic!("mark unread sends the cursor back");
        };
        assert_eq!(
            (channel_id, s, message),
            (target.channel, segment, target.message)
        );
        let Some(MessageAction::Copy(text)) = message_action(COPY_TEXT, &target, None) else {
            panic!("copy hands the text over");
        };
        assert_eq!(text, "hi");
        let Some(MessageAction::React(pending)) = message_action(REACT, &target, None) else {
            panic!("react opens the picker");
        };
        let LogicCommand::Channel {
            channel_id,
            command:
                MurmureCommand::React {
                    at,
                    message,
                    emoji,
                    now,
                },
        } = pending.command(ambre_bus::Emoji::from("\u{1F680}"), Timestamp(9))
        else {
            panic!("a pick reacts to the message");
        };
        assert_eq!(
            (channel_id, at, message, emoji.as_str(), now),
            (
                target.channel,
                MessageAt::Segment(segment),
                target.message,
                "\u{1F680}",
                Timestamp(9)
            )
        );
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

    /// The list holds each chord of the table where it acts anywhere, then
    /// each key of a conversation where its region takes it (`MURMURE-540`).
    #[test]
    fn the_key_list_holds_every_chord_and_every_key_of_a_conversation() {
        let list = key_list(&Locale::En);
        let chords = COMMANDS.iter().flat_map(|c| c.keybindings).count();
        assert_eq!(list.len(), chords + REGION_KEYS.len());
        let palette = KeyRow {
            keys: KeyBinding::primary(KeyChar::K),
            action: "Command palette".to_owned(),
            place: KeyPlace::Anywhere,
        };
        assert_eq!(list[0], palette);
        let (anywhere, region) = list.split_at(chords);
        assert!(anywhere.iter().all(|row| row.place == KeyPlace::Anywhere));
        assert!(region.iter().all(|row| row.place != KeyPlace::Anywhere));
        let also = KeyRow {
            keys: KeyBinding::new(Modifiers::ALT, Key::Enter),
            action: "Also send the reply to the channel".to_owned(),
            place: KeyPlace::ThreadComposer,
        };
        assert!(region.contains(&also), "the chord of the box");
        assert_eq!(also.keys.to_string(), "Alt+Enter");
        assert!(ambre_commands::find(COMMANDS, KEYS).is_some());
    }

    /// The shell resolves its chords before a region sees a key. A key of a
    /// conversation that a chord held would never reach it (`MURMURE-541`).
    #[test]
    fn no_key_of_a_conversation_is_a_chord_of_the_table() {
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        for key in REGION_KEYS {
            assert_eq!(keymap.resolve(key.keys), None, "{}", key.keys);
        }
    }

    /// No chord of the table is one the browser services. Those are the
    /// clipboard chords, modern and legacy, and the undo and redo chords.
    /// Nor is it one the browser keeps from the page, such as the tab chords
    /// or the task manager of Chrome (`MURMURE-171`).
    #[test]
    fn no_chord_is_one_the_browser_services() {
        let reserved = [
            KeyBinding::shift(Key::Escape),
            KeyBinding::primary(KeyChar::W),
            KeyBinding::primary(KeyChar::T),
            KeyBinding::primary(KeyChar::N),
            KeyBinding::primary_shift(KeyChar::W),
            KeyBinding::primary_shift(KeyChar::T),
            KeyBinding::primary_shift(KeyChar::N),
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
