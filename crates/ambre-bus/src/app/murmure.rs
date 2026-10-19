//! Murmure data plane: the command a view sends to a channel, the event the
//! engine answers with, and the value types both carry
//! (`features/ambre-spec-murmure-engine.md section 3`).
//!
//! Pure data, like every module of [`crate::app`]: no Loro type crosses the
//! bus (`SYNC-011`). The engine that reads and writes the documents lives in
//! `ambre-murmure`.
//!
//! A channel is a set of documents, never one document (`MURMURE-021`). So the
//! command is addressed to a [`ChannelId`](crate::ChannelId), carried by
//! [`LogicCommand::Channel`](crate::LogicCommand::Channel), and never by
//! [`LogicCommand::App`](crate::LogicCommand::App), whose address is a
//! document (`BUS-027`).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::{
    CategoryId, ChannelDocumentKind, ChannelId, DocumentId, MessageId, MetaId, SegmentId, Seq,
    SpaceId, UserId,
};

/// One document a channel groups, as the relay lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDocument {
    pub document_id: DocumentId,
    pub kind: ChannelDocumentKind,
}

/// Whether one document of a channel holds something past the caller's
/// read cursor. The relay answers a yes or no, never a count (`SRV-110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentActivity {
    pub channel_id: ChannelId,
    pub document_id: DocumentId,
    /// A segment or a thread. A thread counts its mentions alone.
    pub kind: ChannelDocumentKind,
    pub active: bool,
    /// The caller's own read cursor on the document: the sequence number it
    /// has read up to. The client counts against it once it holds the
    /// document.
    pub read: Seq,
    /// The document's last number when the relay answered. A watch of a
    /// document the client never held resumes after it; nothing counts with
    /// it. `None` from a relay that predates it.
    pub last: Option<Seq>,
}

/// Whether a channel holds something new, and how much once the client
/// knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelActivity {
    pub channel_id: ChannelId,
    /// The mark the sidebar shows before any count is known (`MURMURE-119`).
    pub active: bool,
    /// The new messages of the channel, once the loaded document gives the
    /// number (`MURMURE-120`). `None` while only the mark is known.
    pub unread: Option<UnreadCount>,
    /// The new messages that name this user or the channel. A loaded
    /// document counts its marks, so a false summary does not stay
    /// (`MURMURE-104`). `None` while only the mark is known.
    pub mentions: Option<MentionCount>,
}

/// How many messages past the reader's cursor mention them, or the channel
/// (`MURMURE-233`). A type of its own beside [`UnreadCount`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MentionCount(pub u32);

impl MentionCount {
    /// No mention.
    pub const ZERO: Self = Self(0);

    /// The two counts together, held at the top rather than wrapping.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

/// Where a channel the user belongs to lives, as the relay lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelPlace {
    /// A public channel of the space: its catalogue lists it.
    Public(SpaceId),
    /// A private channel of the space: only its members know it.
    Private(SpaceId),
    /// A direct message, in no space.
    Direct,
}

/// Where a message lies: a segment of the channel, or the thread of a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAt {
    /// A segment of the channel.
    Segment(SegmentId),
    /// The thread document of the root message (the Murmure threads spec).
    Thread(MessageId),
}

impl From<SegmentId> for MessageAt {
    fn from(segment: SegmentId) -> Self {
        Self::Segment(segment)
    }
}

/// One channel the user is a member of, answering
/// [`LogicCommand::ListMemberChannels`](crate::LogicCommand::ListMemberChannels).
/// Learning of it opens nothing (`MURMURE-475`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberChannel {
    pub channel_id: ChannelId,
    pub place: ChannelPlace,
}

/// The `meta` of a channel the user is a member of, as the relay's listing
/// names it. The Logic thread watches a channel it never opened through it
/// (the Murmure notifications spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMeta {
    pub channel_id: ChannelId,
    pub meta: MetaId,
    /// The last number the relay gave in `meta`, none from a relay that
    /// lists none. A watch resumes after it.
    pub last: Option<Seq>,
}

/// The name a channel not yet open shows, read from the name sealed beside
/// its `meta`. The name its `meta` holds replaces it once the channel opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelName {
    pub channel_id: ChannelId,
    pub name: String,
}

/// The display names of a direct message's other members, as the relay
/// lists them. The view composes a name from them for a direct message
/// nobody named, and never stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectPeers {
    pub channel_id: ChannelId,
    pub names: Vec<String>,
}

/// How many messages of a channel are past the reader's cursor. A distinct
/// type, so a count of mentions never takes its place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnreadCount(pub u32);

impl UnreadCount {
    /// Nothing unread.
    pub const ZERO: Self = Self(0);

    /// The two counts together, held at the top rather than wrapping.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Whether nothing is unread.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

/// A wall-clock instant in Unix milliseconds.
///
/// Display only: a peer's clock is untrusted, so no rule orders or refuses a
/// message on it (`ambre-spec-murmure-schema.md section 2`). The view stamps
/// it, because the Logic thread reads no clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub u64);

/// What a mention names: one member, or every member of the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MentionTarget {
    /// One member, by user identifier.
    User(UserId),
    /// The channel itself, an `@channel`.
    Channel,
}

/// One mention over the body of a message: what it names, and the run of
/// text that shows it (`MURMURE-100`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub target: MentionTarget,
    /// In chars of the body, as the composer drew the name.
    pub range: Range<usize>,
}

/// How a run of a message body reads (`MURMURE-194`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextStyle {
    Bold,
    Italic,
    /// Inline code: a marker typed in it stays text (`MURMURE-198`).
    Code,
    /// A link to an `http` or `https` address (`MURMURE-561`).
    Link(String),
}

/// One style over the body of a message, and the run of text it covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StyleMark {
    pub style: TextStyle,
    /// In chars of the body.
    pub range: Range<usize>,
}

/// A message the view hands the engine to send.
///
/// The engine writes each mention as a `mention` mark over its range, and
/// the summary lists what the marks name (`MURMURE-101`). Each style is a
/// mark of its own (`MURMURE-562`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDraft {
    /// The body, as the author typed it.
    pub text: String,
    /// Who the body names, in the order the author typed them.
    pub mentions: Vec<Mention>,
    /// How its runs read, in text order.
    pub styles: Vec<StyleMark>,
}

/// A send the Logic thread holds until its author confirms the `@channel`
/// in it (`MURMURE-231`). The view sends it again, or gives the draft back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSend {
    pub draft: MessageDraft,
    /// The thread a reply goes to, `None` for a message of the channel.
    pub reply: Option<HeldReply>,
}

/// The thread of a held reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldReply {
    pub root: MessageId,
    /// The reply also goes to the channel (`MURMURE-109`).
    pub also_in_channel: bool,
}

/// A command for an open channel, UI thread -> Logic thread.
///
/// Carried by [`LogicCommand::Channel`](crate::LogicCommand::Channel). The
/// variants are the ones the engine builds today. The spec lists the others,
/// each with the Story that brings it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MurmureCommand {
    /// Append a message to the active segment, under this session's user
    /// (`MURMURE-080`).
    SendMessage {
        draft: MessageDraft,
        /// The instant the author sent it, stamped by the view.
        now: Timestamp,
        /// The author confirmed the `@channel` the Logic thread asked about
        /// (`MURMURE-231`). `false` on a first send.
        confirmed: bool,
    },
    /// Load the final segment listed before `segment`, because the view
    /// scrolled past the oldest message it holds (`MURMURE-023`).
    LoadOlder { segment: SegmentId },
    /// The view shows the latest message of the channel: move the read
    /// cursor to it (`MURMURE-115`). A relay record, never a document write.
    MarkRead,
    /// Show the channel unread from `message` on: move the cursor of its
    /// segment to just before it (`MURMURE-117`). A relay record, never a
    /// document write.
    MarkUnread {
        segment: SegmentId,
        message: MessageId,
    },
    /// The view shows the last reply of `thread`: move its read cursor to
    /// it (`MURMURE-293`). A relay record, never a document write.
    MarkThreadRead { thread: DocumentId },
    /// Name a direct message, or clear its name with `None` (`MURMURE-054`,
    /// `MURMURE-055`). A name of blanks alone clears it too. Any member may,
    /// and every member reads it.
    Name { name: Option<String> },
    /// React to `message` with `emoji`, under this session's user
    /// (`MURMURE-092`). A reaction already there writes nothing. A reply
    /// takes it in its thread (`MURMURE-485`).
    React {
        at: MessageAt,
        message: MessageId,
        emoji: String,
        /// The instant the member reacted, stamped by the view.
        now: Timestamp,
    },
    /// Remove this session's own reaction `emoji` from `message`
    /// (`MURMURE-096`). The entry of another member stays (`MURMURE-097`).
    Unreact {
        at: MessageAt,
        message: MessageId,
        emoji: String,
    },
    /// Replace the body of this session's own `message` with `draft`, and
    /// mark it edited (`MURMURE-084`). Its author, identity and reactions
    /// stay (`MURMURE-086`). A reply takes it in its thread (`MURMURE-485`).
    EditMessage {
        at: MessageAt,
        message: MessageId,
        draft: MessageDraft,
        /// The instant the author edited, stamped by the view. A locked
        /// segment takes the edit in an amendments document created then.
        now: Timestamp,
    },
    /// Delete this session's own `message`: it reads deleted, with an empty
    /// body (`MURMURE-087`). Its thread stays (`MURMURE-090`). A reply
    /// takes it in its thread (`MURMURE-485`).
    DeleteMessage {
        at: MessageAt,
        message: MessageId,
        /// The instant the author deleted, stamped by the view.
        now: Timestamp,
    },
    /// Reply to `root` in its thread, under this session's user
    /// (`MURMURE-105`). The first reply opens the thread (`MURMURE-107`).
    /// The engine finds the segment that holds `root`.
    Reply {
        root: MessageId,
        draft: MessageDraft,
        /// The reply also goes to the channel, as a copy that names its
        /// thread (`MURMURE-109`).
        also_in_channel: bool,
        /// The instant the author replied, stamped by the view.
        now: Timestamp,
        /// The author confirmed the `@channel` the Logic thread asked about
        /// (`MURMURE-231`). `false` on a first reply.
        confirmed: bool,
    },
    /// The view's tick of the open channel, once a second while it shows
    /// it. It expires the indicators and refreshes this user's presence in
    /// the channel (the Murmure typing and presence spec).
    Tick { now: Timestamp },
    /// A keystroke in the composer of the channel (`MURMURE-124`), or of the
    /// thread under `thread`, its root (`MURMURE-536`). An ephemeral, never
    /// a document write (`MURMURE-125`).
    Typing {
        now: Timestamp,
        thread: Option<MessageId>,
    },
    /// Ask again for a segment the list shows as a band. Its key chain and
    /// its bytes are fetched again from the relay (`MURMURE-514`).
    Retry { segment: SegmentId },
    /// Find the root of `thread`, which a copy names, down the older
    /// segments, and answer [`MurmureEvent::RootFound`] once it is loaded.
    SeekRoot { thread: DocumentId },
}

impl MurmureCommand {
    /// Whether the command can change a document of the channel.
    ///
    /// The same question [`AppCommand::is_mutating`](crate::AppCommand::is_mutating)
    /// answers for a document. A load reads, and a cursor is no document.
    pub fn is_mutating(&self) -> bool {
        match self {
            Self::SendMessage { .. }
            | Self::Name { .. }
            | Self::React { .. }
            | Self::Unreact { .. }
            | Self::EditMessage { .. }
            | Self::DeleteMessage { .. }
            | Self::Reply { .. } => true,
            Self::LoadOlder { .. }
            | Self::MarkRead
            | Self::MarkUnread { .. }
            | Self::MarkThreadRead { .. }
            | Self::Tick { .. }
            | Self::Typing { .. }
            | Self::Retry { .. }
            | Self::SeekRoot { .. } => false,
        }
    }
}

/// One message as the view draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageView {
    /// The identity the author's peer minted (`MURMURE-002`).
    pub id: MessageId,
    /// The user who wrote the message (`MURMURE-003`).
    pub author: UserId,
    /// The instant the author's peer stamped.
    pub ts: Timestamp,
    /// The body, as plain text.
    pub text: String,
    /// The `mention` marks of the body, in text order (`MURMURE-103`).
    pub mentions: Vec<Mention>,
    /// The style marks of the body, in text order (`MURMURE-562`).
    pub styles: Vec<StyleMark>,
    /// One per emoji, in emoji order (`MURMURE-006`).
    pub reactions: Vec<Reaction>,
    /// What the channel announces, `None` for a message a member wrote. A
    /// notice has an empty body (`MURMURE-057`).
    pub notice: Option<Notice>,
    /// The author edited the body (`MURMURE-084`).
    pub edited: bool,
    /// The author deleted the message: its body is empty (`MURMURE-087`).
    pub deleted: bool,
    /// The thread of replies to this message, `None` when it has no reply
    /// (`MURMURE-106`).
    pub thread: Option<ThreadHead>,
    /// The thread a reply also sent to the channel came from, `None` for
    /// any other message (`MURMURE-109`).
    pub also_in_thread: Option<DocumentId>,
}

/// What a root shows of its thread (`MURMURE-106`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadHead {
    /// The thread document its `thread` field names.
    pub document: DocumentId,
    /// The replies the thread document holds, deleted ones aside. Zero
    /// until the document is held.
    pub replies: u32,
    /// The author of the latest reply, `None` until the document is held.
    pub last_replier: Option<UserId>,
}

/// An event of the channel itself, shown in its message list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// A member joins a direct message (`MURMURE-057`).
    MemberAdded {
        /// The member who joins.
        member: UserId,
    },
}

impl MessageView {
    /// The body as the view shows it: each mention of a member reads as
    /// `@` and that member's current name (`MURMURE-098`). A name `name_of`
    /// does not know keeps the text the author typed.
    pub fn shown(&self, name_of: impl Fn(UserId) -> Option<String>) -> String {
        self.shown_at(name_of).0
    }

    /// The body as [`shown`](Self::shown) writes it, and the byte of it
    /// where each char of the text starts, one past the last included. A
    /// char a name took the place of starts past that name (`MURMURE-565`).
    pub fn shown_at(&self, name_of: impl Fn(UserId) -> Option<String>) -> (String, Vec<usize>) {
        let mut shown = String::with_capacity(self.text.len());
        let mut bytes = Vec::with_capacity(self.text.len() + 1);
        let mut at = 0;
        for (i, c) in self.text.chars().enumerate() {
            if i < at {
                bytes.push(shown.len());
                continue;
            }
            bytes.push(shown.len());
            let mention = self.mentions.iter().find(|m| m.range.start == i);
            let named = mention.and_then(|m| match m.target {
                MentionTarget::User(user) => name_of(user).map(|name| (m, name)),
                MentionTarget::Channel => None,
            });
            match named {
                Some((m, name)) => {
                    shown.push('@');
                    shown.push_str(&name);
                    at = m.range.end;
                }
                None => shown.push(c),
            }
        }
        bytes.push(shown.len());
        (shown, bytes)
    }
}

/// The members who reacted to a message with one emoji.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub emoji: String,
    /// Each member once, in identifier order.
    pub users: Vec<UserId>,
}

/// One message a search found, with the segment that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub segment: SegmentId,
    pub message: MessageView,
}

/// What a search found in one open channel, and which part of its history
/// it read (the Murmure search spec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMatches {
    pub channel_id: ChannelId,
    /// The segments searched: the ones the client holds, in list order.
    pub covered: Vec<SegmentId>,
    /// How many segments `meta` lists, searched or not.
    pub listed: usize,
    /// The matches, segment by segment in list order, each segment's in
    /// message order.
    pub matches: Vec<SearchMatch>,
}

/// An event from an open channel, Logic thread -> UI thread.
///
/// Carried by [`UiEvent::Channel`](crate::UiEvent::Channel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MurmureEvent {
    /// These messages of `segment` are new or changed. The view keys them by
    /// [`MessageId`], so a message it already holds is replaced, never
    /// duplicated. One it holds under another segment moves to `segment`.
    /// `messages` keeps the order of the segment.
    Messages {
        segment: SegmentId,
        messages: Vec<MessageView>,
    },
    /// These replies of the thread of `root` are new or changed, keyed as
    /// [`MurmureEvent::Messages`] keys its own. `messages` keeps the order
    /// of the thread document (`MURMURE-105`).
    ThreadMessages {
        root: MessageId,
        thread: DocumentId,
        messages: Vec<MessageView>,
    },
    /// The root of `thread`, which a copy names, lies in `segment`, now
    /// loaded (the Murmure threads spec). Its thread is to open.
    RootFound {
        thread: DocumentId,
        segment: SegmentId,
        root: MessageId,
    },
    /// The root of `thread`, which a copy names, could not be reached (the
    /// Murmure threads spec). Nothing opens.
    RootLost { thread: DocumentId },
    /// The first message past the read cursors, in list order, that another
    /// member wrote. `None` when every held message is read. Sent each time
    /// the count of the channel is known anew (`MURMURE-499`).
    FirstUnread { message: Option<MessageId> },
    /// The channel could not open: the relay refused it, or one of its
    /// documents failed the checks. Nothing of it is held.
    OpenFailed { message: String },
    /// The name the channel's `meta` holds, at the open and after any change
    /// to it. `None` for an unnamed channel: the view composes what it shows
    /// from the members, and stores nothing (`MURMURE-016`).
    Named { name: Option<String> },
    /// This user is no member of the channel any more. They left it from
    /// another client, or the relay refused its open. Nothing of it is held
    /// any more (`MURMURE-468`, `MURMURE-469`).
    Left,
    /// The other members who watch the channel, those of them who type in
    /// it, and those who type in each thread, by its root. Each list runs in
    /// the order of the identifiers. Sent whenever one changes
    /// (`MURMURE-124`, `MURMURE-126`, `MURMURE-536`).
    Watchers {
        watching: Vec<UserId>,
        typing: Vec<UserId>,
        in_threads: Vec<(MessageId, Vec<UserId>)>,
    },
    /// The members of the channel, as the relay lists them: the `@` list of
    /// the composer offers them alone. Sent at the open and on every change.
    Members { members: Vec<crate::MemberInfo> },
    /// The draft mentions the whole channel, which holds `members` members.
    /// Nothing was sent: the view asks the author, then sends `held` again
    /// confirmed (`MURMURE-231`).
    ConfirmChannelMention { members: usize, held: HeldSend },
    /// A member reported the snapshot of `segment` as false, and the relay
    /// dropped it. The view names its author and the reporter; `None` for a
    /// key no member holds any more (`MURMURE-239`).
    SnapshotDropped {
        segment: SegmentId,
        author: Option<UserId>,
        reporter: Option<UserId>,
    },
    /// The older segment the view asked for cannot be read, for `cause`. It
    /// holds the messages from `from` until `to`; `to` is `None` when no
    /// later segment starts (`MURMURE-218`).
    Unreadable {
        segment: SegmentId,
        cause: Unreadable,
        from: Timestamp,
        to: Option<Timestamp>,
    },
}

/// Why a segment cannot be read. The engine names the cause, and the list
/// names the remedy (the Murmure message list spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    /// The key chain did not reach the key of this segment.
    KeyNotReceived,
    /// The member was invited without this history.
    BeforeHistoryBoundary,
    /// The key it names is held, and the segment does not open.
    Damaged,
}

/// A channel key sealed under the key of its space (`MURMURE-157`).
///
/// The Logic thread holds it and never opens it: the space document lives
/// there, and no key material may reach it (`SYNC-039`). Only the Sync
/// worker, which holds the space key, seals and unseals it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedChannelKey(Vec<u8>);

impl SealedChannelKey {
    /// Wrap the sealed bytes the Sync worker produced.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The sealed bytes, as the catalogue stores them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for SealedChannelKey {
    // The length alone: a log line needs no ciphertext.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SealedChannelKey({} bytes)", self.0.len())
    }
}

/// The topic of a channel: a line of text under its name.
///
/// Never empty: a channel with no topic holds no topic at all, so the
/// catalogue stores an absent key rather than an empty string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Topic(String);

impl Topic {
    /// The topic `text` states, or `None` for an empty text.
    pub fn new(text: impl Into<String>) -> Option<Self> {
        let text = text.into();
        (!text.is_empty()).then_some(Self(text))
    }

    /// The text of the topic.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Topic {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text).ok_or("a topic is never empty")
    }
}

impl From<Topic> for String {
    fn from(topic: Topic) -> Self {
        topic.0
    }
}

/// A category of a space, as the view draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryView {
    pub id: CategoryId,
    pub name: String,
    /// The rank among the categories: lower first, ties by identifier.
    pub position: u32,
}

/// One public channel of the catalogue, as the view draws it.
///
/// No key: the view has no use for one, and the sealed key stays in the space
/// document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub channel_id: ChannelId,
    pub name: String,
    pub topic: Option<Topic>,
    /// Absent for a channel outside every category.
    pub category: Option<CategoryId>,
    /// The rank in the catalogue: lower first, ties by identifier.
    pub position: u32,
}

/// A public channel the view asks a space to create and list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicChannelDraft {
    /// Minted by the view, so it can follow the channel before it exists.
    pub channel_id: ChannelId,
    pub name: String,
    pub topic: Option<Topic>,
    /// Absent for a channel outside every category.
    pub category: Option<CategoryId>,
    /// The rank in the catalogue: lower first, ties by identifier.
    pub position: u32,
}

/// Whether a member of a space is online, as the member list marks it.
///
/// Away and busy join this set with the `Presence states` Story.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presence {
    /// A connection of the member holds the space open.
    Online,
    /// No connection of the member refreshed their presence in time.
    Offline,
}

/// One member of a space, as the member list draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberView {
    pub user_id: UserId,
    pub display_name: String,
    pub presence: Presence,
}

/// A private channel the view asks a space to create. It is listed nowhere,
/// so it has no topic, category nor rank in the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateChannelDraft {
    /// Minted by the view, so it can follow the channel before it exists.
    pub channel_id: ChannelId,
    pub name: String,
}

/// A member of a direct message, other than this user, with the exchange key
/// the user confirmed for them (`ambre-spec-key-verification.md`).
///
/// The key is public. The Sync worker fetches the member's key again and
/// wraps the channel key only when the two match. So a key the relay swapped
/// in never receives a wrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectMember {
    pub user_id: UserId,
    pub exchange_pub_key: [u8; 32],
}

/// A direct message the view asks for: this user and `members`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMessageDraft {
    /// Minted by the view. It names the request, and the channel too when
    /// the relay creates one. An existing direct message keeps its own.
    pub channel_id: ChannelId,
    /// Every member but this user.
    pub members: Vec<DirectMember>,
}

/// The outcome of a direct message request, Logic thread -> UI thread.
///
/// Carried by [`UiEvent::DirectMessage`](crate::UiEvent::DirectMessage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectMessageEvent {
    /// The direct message of these members, created or found
    /// (`MURMURE-051`). It is opening: its events follow under `channel_id`,
    /// which differs from the request's when the direct message existed.
    Opened { channel_id: ChannelId },
    /// The request failed. Nothing changed on the relay.
    Failed { message: String },
}

/// A command for an open space, UI thread -> Logic thread.
///
/// Carried by [`LogicCommand::Space`](crate::LogicCommand::Space). A space is
/// one document, and its catalogue lists its public channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceCommand {
    /// Add a category to the catalogue.
    AddCategory {
        category_id: CategoryId,
        name: String,
        /// The rank among the categories: lower first, ties by identifier.
        position: u32,
    },
    /// Create a public channel, then list it in the catalogue with its key
    /// sealed under the space key (`MURMURE-037`).
    CreatePublicChannel {
        draft: PublicChannelDraft,
        /// The instant of the creation, stamped by the view.
        now: Timestamp,
    },
    /// Create a private channel of the space, with this user as its owner and
    /// only member (`MURMURE-048`). The catalogue does not change
    /// (`MURMURE-050`).
    CreatePrivateChannel {
        draft: PrivateChannelDraft,
        /// The instant of the creation, stamped by the view.
        now: Timestamp,
    },
    /// Join a public channel of the catalogue, with no other member online
    /// (`MURMURE-039`).
    JoinChannel { channel_id: ChannelId },
    /// Delete a channel of the space this user owns, with every document it
    /// groups. Once the relay answers, the client drops it, and a public one
    /// leaves the catalogue (`MURMURE-462`, `MURMURE-247`). A refusal keeps
    /// it whole (`MURMURE-463`).
    DeleteChannel { channel_id: ChannelId },
    /// Leave a channel of the space. Once the relay answers, the client drops
    /// it with its documents and its keys (`MURMURE-467`). A public one stays
    /// in the catalogue, and a private one's key is owed to the members who
    /// stay (`MURMURE-059`). A refusal keeps it whole.
    LeaveChannel { channel_id: ChannelId },
    /// Send this user's presence on the space, and turn offline the members
    /// whose presence expired. The view sends it once per refresh period
    /// (`ambre-spec-budgets.md section 8`): the Logic thread reads no clock.
    RefreshPresence {
        /// The view's clock. It never travels: it ages the members.
        now: Timestamp,
    },
}

/// An event from an open space, Logic thread -> UI thread.
///
/// Carried by [`UiEvent::Space`](crate::UiEvent::Space).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceEvent {
    /// The space and its catalogue, whole, after an open or any change.
    Catalogue {
        /// Absent when the space has no name.
        name: Option<String>,
        categories: Vec<CategoryView>,
        channels: Vec<CatalogueEntry>,
    },
    /// The members of the space, whole, after an open or any change: the
    /// online ones first, then the others, each group by display name.
    Members { members: Vec<MemberView> },
    /// The relay holds the new channel, and the catalogue lists it when it
    /// is public.
    ChannelCreated { channel_id: ChannelId },
    /// The user is now a member of the channel, which the view may open.
    Joined { channel_id: ChannelId },
    /// The relay deleted the channel, and this client holds nothing of it.
    ChannelDeleted { channel_id: ChannelId },
    /// The user left the channel, and this client holds nothing of it.
    ChannelLeft { channel_id: ChannelId },
    /// A creation, a join, a delete or a leave failed. Nothing changed on the
    /// relay.
    ChannelFailed {
        channel_id: ChannelId,
        message: String,
    },
    /// The space could not open. Nothing of it is held.
    OpenFailed { message: String },
    /// The user is no longer a member of the space: they left it, an owner
    /// removed them, or it was deleted. Nothing of it is held.
    Left,
}

#[cfg(test)]
mod tests {
    use super::{MurmureCommand, Topic};
    use ambre_core::SegmentId;

    /// A retry fetches again what the channel already holds: it writes no
    /// document, so a read-only member sends it too.
    #[test]
    fn a_retry_writes_no_document() {
        let retry = MurmureCommand::Retry {
            segment: SegmentId::from_bytes([7; 16]),
        };
        assert!(!retry.is_mutating());
    }

    /// An empty topic is no topic: it cannot be built, nor decoded from the
    /// wire, so the catalogue never stores an empty string.
    #[test]
    fn a_topic_is_never_empty() {
        assert_eq!(Topic::new(""), None);
        let figures = Topic::new("the weekly figures").expect("a topic");
        assert_eq!(figures.as_str(), "the weekly figures");

        let bytes = postcard::to_allocvec(&figures).expect("encode");
        assert_eq!(postcard::from_bytes::<Topic>(&bytes), Ok(figures));
        let empty = postcard::to_allocvec("").expect("encode");
        assert!(postcard::from_bytes::<Topic>(&empty).is_err());
    }
}
