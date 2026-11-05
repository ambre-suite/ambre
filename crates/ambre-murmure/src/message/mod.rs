//! Murmure data plane: the command a view sends to a channel, the event the
//! engine answers with, and the value types both carry
//! (`features/ambre-spec-murmure-engine.md section 3`). It lives in the
//! application's crate, and the Sync worker never builds against it
//! (`BUS-032`, `ARCH-070`). The channel types of the service plane stay in
//! `ambre-bus`, where the worker reads them.
//!
//! Pure data: no Loro type crosses the bus (`SYNC-011`).
//!
//! A channel is a set of documents, never one document (`MURMURE-021`). So the
//! command is addressed to an `Id<Channel>`, carried by `LogicCommand::Channel`
//! through the [`Murmure`] plane, and never by `LogicCommand::App`, whose
//! address is a document (`BUS-027`).

use ambre_core::index::{IdxVec, Len, Pos, Span};
use ambre_core::{Byte, Char, Count, CountKind, Rank, RankKind, Validated};

use ambre_core::id::{Category, Channel, Document, Id, Message, Segment, User};

use ambre_bus::{ChannelTopic, DirectMember, Emoji};

/// Where a message lies: a segment of the channel, or the thread of a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAt {
    /// A segment of the channel.
    Segment(Id<Segment>),
    /// The thread document of the root message (the Murmure threads spec).
    Thread(Id<Message>),
}

impl From<Id<Segment>> for MessageAt {
    fn from(segment: Id<Segment>) -> Self {
        Self::Segment(segment)
    }
}

pub use ambre_core::{Authored, Instant};

/// What a mention names: one member, or every member of the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MentionTarget {
    /// One member, by user identifier.
    User(Id<User>),
    /// The channel itself, an `@channel`.
    Channel,
}

/// One mention over the body of a message: what it names, and the run of
/// text that shows it (`MURMURE-100`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub target: MentionTarget,
    /// In chars of the body, as the composer drew the name.
    pub range: Span<Char>,
}

/// How a run of a message body reads (`MURMURE-194`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextStyle {
    Bold,
    Italic,
    /// Inline code: a marker typed in it stays text (`MURMURE-198`).
    Code,
    /// A link to an `http` or `https` address (`MURMURE-561`).
    Link(ambre_core::LinkUrl),
    /// A block of code: its lines, the fences gone. A marker typed in it
    /// stays text (`MURMURE-198`).
    CodeBlock,
    /// A quote: its lines, each with its line break.
    Quote,
    /// A list: one item per line, each with its line break.
    List,
}

/// One style over the body of a message, and the run of text it covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StyleMark {
    pub style: TextStyle,
    /// In chars of the body.
    pub range: Span<Char>,
}

/// A message the view hands the engine to send.
///
/// The engine writes each mention as a `mention` mark over its range, and
/// the summary lists what the marks name (`MURMURE-101`). Each style is a
/// mark of its own (`MURMURE-562`). `Debug` prints the length of the body
/// alone, so a log of a send, an edit or a reply carries none of it.
#[derive(Clone, PartialEq, Eq)]
pub struct MessageDraft {
    /// The body, as the author typed it.
    pub text: String,
    /// Who the body names, in the order the author typed them.
    pub mentions: Vec<Mention>,
    /// How its runs read, in text order.
    pub styles: Vec<StyleMark>,
}

impl std::fmt::Debug for MessageDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageDraft")
            .field("text", &ambre_core::RedactedChars::of(&self.text))
            .field("mentions", &self.mentions)
            .field("styles", &self.styles)
            .finish()
    }
}

// A file's media type is shown and used to open the file, and never sent to
// the relay (`SYNC-053`). The three types live in `ambre-core`, which the
// relay's media routes share. A message names its file by the address the
// Sync worker gave it, read back through `Validated::<ContentHash>::parse`.
pub use ambre_core::{ContentHash, MediaType, Octets};

/// A file a message carries: its address, never its bytes (`MURMURE-017`).
/// `Debug` prints the length of the file's name alone.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Attachment {
    pub media_ref: Validated<ContentHash>,
    /// The file's name, as its author's file system gave it.
    pub name: String,
    pub media_type: MediaType,
    pub size: Count<Octets>,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attachment")
            .field("media_ref", &self.media_ref)
            .field("name", &ambre_core::RedactedChars::of(&self.name))
            .field("media_type", &self.media_type)
            .field("size", &self.size)
            .finish()
    }
}

/// An image a message carries, decoded off the render thread at the size
/// of its preview (`MURMURE-135`). A still image has one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// The size of every frame.
    pub size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    /// Each frame in play order, never empty.
    pub frames: Vec<PictureFrame>,
}

/// One frame of a [`Picture`]: non-premultiplied RGBA8, row-major, `width *
/// height * 4` bytes of its size, shown for `delay`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureFrame {
    pub pixels: ambre_core::Payload<ambre_core::RgbaPixels>,
    pub delay: TimeSpan<Ms>,
}

impl Picture {
    /// The bytes its frames hold.
    pub fn bytes(&self) -> Count<Octets> {
        self.frames.iter().map(|f| f.pixels.len()).sum()
    }
}

/// A send the Logic thread holds until its author confirms the `@channel`
/// in it (`MURMURE-231`). The view sends it again, or gives the draft back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSend {
    pub draft: MessageDraft,
    /// The files the message carries, each already uploaded (`MURMURE-132`).
    pub attachments: Vec<Attachment>,
    /// The thread a reply goes to, `None` for a message of the channel.
    pub reply: Option<HeldReply>,
}

/// Where a reply goes besides its thread (`MURMURE-109`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplyEcho {
    /// The reply stays in its thread.
    #[default]
    ThreadOnly,
    /// The reply also goes to the channel, as a copy that names its thread.
    AlsoInChannel,
}

/// The thread of a held reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldReply {
    pub root: Id<Message>,
    /// Whether the reply also goes to the channel (`MURMURE-109`).
    pub also_in_channel: ReplyEcho,
}

/// Whether the author of a send confirmed the `@channel` it holds, once
/// the Logic thread asked (`MURMURE-231`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Confirmation {
    #[default]
    Unconfirmed,
    Confirmed,
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
        /// The files it carries. The view names only files the Sync worker
        /// answered uploaded, under the channel's `meta` (`MURMURE-132`).
        attachments: Vec<Attachment>,
        /// The instant the author sent it, stamped by the view.
        now: Instant<Authored>,
        /// Whether the author confirmed the `@channel` the Logic thread asked
        /// about (`MURMURE-231`). [`Confirmation::Unconfirmed`] on a first send.
        confirmed: Confirmation,
    },
    /// Load the final segment listed before `segment`, because the view
    /// scrolled past the oldest message it holds (`MURMURE-023`).
    LoadOlder { segment: Id<Segment> },
    /// The view shows the latest message of the channel: move the read
    /// cursor to it (`MURMURE-115`). A relay record, never a document write.
    MarkRead,
    /// Show the channel unread from `message` on: move the cursor of its
    /// segment to just before it (`MURMURE-117`). A relay record, never a
    /// document write.
    MarkUnread {
        segment: Id<Segment>,
        message: Id<Message>,
    },
    /// The view shows the last reply of `thread`: move its read cursor to
    /// it (`MURMURE-293`). A relay record, never a document write.
    MarkThreadRead { thread: Id<Document> },
    /// Name a direct message, or clear its name with `None` (`MURMURE-054`,
    /// `MURMURE-055`). A name of blanks alone clears it too. Any member may,
    /// and every member reads it.
    Name {
        name: Option<ambre_core::Validated<ambre_core::Name>>,
    },
    /// React to `message` with `emoji`, under this session's user
    /// (`MURMURE-092`). A reaction already there writes nothing. A reply
    /// takes it in its thread (`MURMURE-485`).
    React {
        at: MessageAt,
        message: Id<Message>,
        emoji: Emoji,
        /// The instant the member reacted, stamped by the view.
        now: Instant<Authored>,
    },
    /// Remove this session's own reaction `emoji` from `message`
    /// (`MURMURE-096`). The entry of another member stays (`MURMURE-097`).
    Unreact {
        at: MessageAt,
        message: Id<Message>,
        emoji: Emoji,
    },
    /// Replace the body of this session's own `message` with `draft`, and
    /// mark it edited (`MURMURE-084`). Its author, identity and reactions
    /// stay (`MURMURE-086`). A reply takes it in its thread (`MURMURE-485`).
    EditMessage {
        at: MessageAt,
        message: Id<Message>,
        draft: MessageDraft,
        /// The instant the author edited, stamped by the view. A locked
        /// segment takes the edit in an amendments document created then.
        now: Instant<Authored>,
    },
    /// Delete this session's own `message`: it reads deleted, with an empty
    /// body (`MURMURE-087`). Its thread stays (`MURMURE-090`). A reply
    /// takes it in its thread (`MURMURE-485`).
    DeleteMessage {
        at: MessageAt,
        message: Id<Message>,
        /// The instant the author deleted, stamped by the view.
        now: Instant<Authored>,
    },
    /// Reply to `root` in its thread, under this session's user
    /// (`MURMURE-105`). The first reply opens the thread (`MURMURE-107`).
    /// The engine finds the segment that holds `root`.
    Reply {
        root: Id<Message>,
        draft: MessageDraft,
        /// The reply also goes to the channel, as a copy that names its
        /// thread (`MURMURE-109`).
        also_in_channel: ReplyEcho,
        /// The instant the author replied, stamped by the view.
        now: Instant<Authored>,
        /// Whether the author confirmed the `@channel` the Logic thread asked
        /// about (`MURMURE-231`). [`Confirmation::Unconfirmed`] on a first
        /// reply.
        confirmed: Confirmation,
    },
    /// The view's tick of the open channel, once a second while it shows
    /// it. It expires the indicators and refreshes this user's presence in
    /// the channel (the Murmure typing and presence spec).
    Tick { now: Instant<Authored> },
    /// A keystroke in the composer of the channel (`MURMURE-124`), or of the
    /// thread under `thread`, its root (`MURMURE-536`). An ephemeral, never
    /// a document write (`MURMURE-125`).
    Typing {
        now: Instant<Authored>,
        thread: Option<Id<Message>>,
    },
    /// Ask again for a segment the list shows as a band. Its key chain and
    /// its bytes are fetched again from the relay (`MURMURE-514`).
    Retry { segment: Id<Segment> },
    /// Find the root of `thread`, which a copy names, down the older
    /// segments, and answer [`MurmureEvent::RootFound`] once it is loaded.
    SeekRoot { thread: Id<Document> },
    /// Upload a file for the composer, under the channel's `meta`, through
    /// the shared media flow (`MURMURE-132`). The answer is the
    /// `MediaUploaded` of request `id`, or an `ApiError` naming it. The view
    /// holds no document of the channel, so the Logic thread names it.
    UploadFile {
        id: ambre_core::Seq<ambre_bus::Request>,
        data: ambre_core::Payload<ambre_core::MediaBytes>,
    },
    /// Fetch the file `media_ref` of a message, stored under the channel's
    /// `meta`. The answer is the `MediaDownloaded` of request `id`.
    FetchFile {
        id: ambre_core::Seq<ambre_bus::Request>,
        media_ref: Validated<ContentHash>,
    },
    /// Fetch the image `media_ref` of a message and decode it for its
    /// preview. The answer is [`MurmureEvent::Picture`].
    ShowPicture { media_ref: Validated<ContentHash> },
}

impl MurmureCommand {
    /// Whether the command can change a document of the channel.
    ///
    /// The same question [`Mutation::is_mutating`](crate::Mutation::is_mutating)
    /// answers for a document. A load reads, and a cursor is no document.
    pub fn is_mutating(&self) -> bool {
        match self {
            Self::SendMessage { .. }
            | Self::Name { .. }
            | Self::React { .. }
            | Self::Unreact { .. }
            | Self::EditMessage { .. }
            | Self::DeleteMessage { .. }
            | Self::Reply { .. }
            // A file uploads only for a message to carry it.
            | Self::UploadFile { .. } => true,
            Self::LoadOlder { .. }
            | Self::FetchFile { .. }
            | Self::ShowPicture { .. }
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

ambre_core::define_flag! {
    /// Whether the author edited the body of a message (`MURMURE-084`).
    pub enum Revision {
        /// The body as the author first sent it.
        Original,
        /// The author edited the body.
        Edited,
    }

    /// Whether the author deleted a message (`MURMURE-087`).
    pub enum Deletion {
        /// The message shows its body.
        Live,
        /// The author deleted the message: its body is empty.
        Deleted,
    }
}

/// One message as the view draws it. `Debug` prints the length of the body
/// alone, so a log of a message list or of a search carries none of it.
#[derive(Clone, PartialEq, Eq)]
pub struct MessageView {
    /// The identity the author's peer minted (`MURMURE-002`).
    pub id: Id<Message>,
    /// The user who wrote the message (`MURMURE-003`).
    pub author: Id<User>,
    /// The instant the author's peer stamped.
    pub ts: Instant<Authored>,
    /// The body, as plain text.
    pub text: String,
    /// The `mention` marks of the body, in text order (`MURMURE-103`).
    pub mentions: Vec<Mention>,
    /// The style marks of the body, in text order (`MURMURE-562`).
    pub styles: Vec<StyleMark>,
    /// The files it carries, in the order its author attached them. A
    /// deleted message carries none (`MURMURE-240`).
    pub attachments: Vec<Attachment>,
    /// One per emoji, in emoji order (`MURMURE-006`).
    pub reactions: Vec<Reaction>,
    /// What the channel announces, `None` for a message a member wrote. A
    /// notice has an empty body (`MURMURE-057`).
    pub notice: Option<Notice>,
    /// Whether the author edited the body (`MURMURE-084`).
    pub edited: Revision,
    /// Whether the author deleted the message (`MURMURE-087`).
    pub deleted: Deletion,
    /// The thread of replies to this message, `None` when it has no reply
    /// (`MURMURE-106`).
    pub thread: Option<ThreadHead>,
    /// The thread a reply also sent to the channel came from, `None` for
    /// any other message (`MURMURE-109`).
    pub also_in_thread: Option<Id<Document>>,
}

impl std::fmt::Debug for MessageView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageView")
            .field("id", &self.id)
            .field("author", &self.author)
            .field("ts", &self.ts)
            .field("text", &ambre_core::RedactedChars::of(&self.text))
            .field("mentions", &self.mentions)
            .field("styles", &self.styles)
            .field("attachments", &self.attachments)
            .field("reactions", &self.reactions)
            .field("notice", &self.notice)
            .field("edited", &self.edited)
            .field("deleted", &self.deleted)
            .field("thread", &self.thread)
            .field("also_in_thread", &self.also_in_thread)
            .finish()
    }
}

/// How many replies a thread holds, deleted ones aside.
pub enum Replies {}

impl CountKind for Replies {
    type Repr = u32;
    const NAME: &'static str = "ReplyCount";
}

/// How many segments a channel lists.
pub enum Segments {}

impl CountKind for Segments {
    type Repr = u32;
    const NAME: &'static str = "SegmentCount";
}

/// What a root shows of its thread (`MURMURE-106`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadHead {
    /// The thread document its `thread` field names.
    pub document: Id<Document>,
    /// The replies the thread document holds, deleted ones aside. Zero
    /// until the document is held.
    pub replies: Count<Replies>,
    /// The author of the latest reply, `None` until the document is held.
    pub last_replier: Option<Id<User>>,
}

/// An event of the channel itself, shown in its message list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// A member joins a direct message (`MURMURE-057`).
    MemberAdded {
        /// The member who joins.
        member: Id<User>,
    },
}

impl MessageView {
    /// The body as the view shows it: each mention of a member reads as
    /// `@` and that member's current name (`MURMURE-098`). A name `name_of`
    /// does not know keeps the text the author typed.
    pub fn shown(
        &self,
        name_of: impl Fn(Id<User>) -> Option<ambre_core::Validated<ambre_core::DisplayName>>,
    ) -> String {
        self.shown_at(name_of).0
    }

    /// The body as [`shown`](Self::shown) writes it, and the byte of it
    /// where each char of the text starts, one past the last included. A
    /// char a name took the place of starts past that name (`MURMURE-565`).
    pub fn shown_at(
        &self,
        name_of: impl Fn(Id<User>) -> Option<ambre_core::Validated<ambre_core::DisplayName>>,
    ) -> (String, IdxVec<Char, Pos<Byte>>) {
        let mut shown = String::with_capacity(self.text.len());
        let mut bytes = IdxVec::<Char, _>::new();
        let mut at = Pos::<Char>::ZERO;
        let chars = self.text.chars().enumerate();
        for (i, c) in chars.map(|(i, c)| (Len::<Char>::of(i).as_end(), c)) {
            if i < at {
                bytes.push(Pos::<Byte>::from_usize(shown.len()));
                continue;
            }
            bytes.push(Pos::<Byte>::from_usize(shown.len()));
            let mention = self.mentions.iter().find(|m| m.range.start == i);
            let named = mention.and_then(|m| match m.target {
                MentionTarget::User(user) => name_of(user).map(|name| (m, name)),
                MentionTarget::Channel => None,
            });
            match named {
                Some((m, name)) => {
                    shown.push('@');
                    shown.push_str(name.as_str());
                    at = m.range.end;
                }
                None => shown.push(c),
            }
        }
        bytes.push(Pos::<Byte>::from_usize(shown.len()));
        (shown, bytes)
    }
}

/// The members who reacted to a message with one emoji.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub emoji: Emoji,
    /// Each member once, in identifier order.
    pub users: Vec<Id<User>>,
}

/// One message a search found, with the segment that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub segment: Id<Segment>,
    pub message: MessageView,
}

/// Who registered the segment a close moves to. The Sync worker answers it,
/// so it lives on the event plane, and the worker names no app message.
pub use ambre_bus::Successor;
use ambre_core::{Ms, TimeSpan};

/// What a search found in one open channel, and which part of its history
/// it read (the Murmure search spec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMatches {
    pub channel_id: Id<Channel>,
    /// The segments searched: the ones the client holds, in list order.
    pub covered: Vec<Id<Segment>>,
    /// How many segments `meta` lists, searched or not.
    pub listed: Count<Segments>,
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
    /// [`Id<Message>`], so a message it already holds is replaced, never
    /// duplicated. One it holds under another segment moves to `segment`.
    /// `messages` keeps the order of the segment.
    Messages {
        segment: Id<Segment>,
        messages: Vec<MessageView>,
    },
    /// These replies of the thread of `root` are new or changed, keyed as
    /// [`MurmureEvent::Messages`] keys its own. `messages` keeps the order
    /// of the thread document (`MURMURE-105`).
    ThreadMessages {
        root: Id<Message>,
        thread: Id<Document>,
        messages: Vec<MessageView>,
    },
    /// The root of `thread`, which a copy names, lies in `segment`, now
    /// loaded (the Murmure threads spec). Its thread is to open.
    RootFound {
        thread: Id<Document>,
        segment: Id<Segment>,
        root: Id<Message>,
    },
    /// The root of `thread`, which a copy names, could not be reached (the
    /// Murmure threads spec). Nothing opens.
    RootLost { thread: Id<Document> },
    /// The first message past the read cursors, in list order, that another
    /// member wrote. `None` when every held message is read. Sent each time
    /// the count of the channel is known anew (`MURMURE-499`).
    FirstUnread { message: Option<Id<Message>> },
    /// The channel could not open: the relay refused it, or one of its
    /// documents failed the checks. Nothing of it is held.
    OpenFailed { message: String },
    /// The name the channel's `meta` holds, at the open and after any change
    /// to it. `None` for an unnamed channel: the view composes what it shows
    /// from the members, and stores nothing (`MURMURE-016`).
    Named {
        name: Option<ambre_core::Validated<ambre_core::Name>>,
    },
    /// This user is no member of the channel any more. They left it from
    /// another client, or the relay refused its open. Nothing of it is held
    /// any more (`MURMURE-468`, `MURMURE-469`).
    Left,
    /// The other members who watch the channel, those of them who type in
    /// it, and those who type in each thread, by its root. Each list runs in
    /// the order of the identifiers. Sent whenever one changes
    /// (`MURMURE-124`, `MURMURE-126`, `MURMURE-536`).
    Watchers {
        watching: Vec<Id<User>>,
        typing: Vec<Id<User>>,
        in_threads: Vec<(Id<Message>, Vec<Id<User>>)>,
    },
    /// The members of the channel, as the relay lists them: the `@` list of
    /// the composer offers them alone. Sent at the open and on every change.
    Members { members: Vec<ambre_bus::MemberInfo> },
    /// The draft mentions the whole channel, which holds `members` members.
    /// Nothing was sent: the view asks the author, then sends `held` again
    /// confirmed (`MURMURE-231`).
    ConfirmChannelMention {
        members: Count<ambre_core::Members>,
        held: HeldSend,
    },
    /// A member reported the snapshot of `segment` as false, and the relay
    /// dropped it. The view names its author and the reporter; `None` for a
    /// key no member holds any more (`MURMURE-239`).
    SnapshotDropped {
        segment: Id<Segment>,
        author: Option<Id<User>>,
        reporter: Option<Id<User>>,
    },
    /// The older segment the view asked for cannot be read, for `cause`. It
    /// holds the messages from `from` until `to`; `to` is `None` when no
    /// later segment starts (`MURMURE-218`).
    Unreadable {
        segment: Id<Segment>,
        cause: Unreadable,
        from: Instant<Authored>,
        to: Option<Instant<Authored>>,
    },
    /// The image `media_ref`, decoded for its preview, answering a
    /// `ShowPicture`. `None` when it could not be fetched or decoded: the
    /// view keeps its chip alone.
    Picture {
        media_ref: Validated<ContentHash>,
        picture: Option<Picture>,
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

/// The order of the categories and channels in the catalogue of a space:
/// lower first, ties by identifier. Any `u32` is a rank.
pub enum CatalogPosition {}

impl RankKind for CatalogPosition {
    type Repr = u32;
    const NAME: &'static str = "CatalogPosition";
}

/// A category of a space, as the view draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryView {
    pub id: Id<Category>,
    pub name: ambre_core::Validated<ambre_core::Name>,
    /// The rank among the categories: lower first, ties by identifier.
    pub position: Rank<CatalogPosition>,
}

/// One public channel of the catalogue, as the view draws it.
///
/// No key: the view has no use for one, and the sealed key stays in the space
/// document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub channel_id: Id<Channel>,
    pub name: ambre_core::Validated<ambre_core::Name>,
    pub topic: Option<Validated<ChannelTopic>>,
    /// Absent for a channel outside every category.
    pub category: Option<Id<Category>>,
    /// The rank in the catalogue: lower first, ties by identifier.
    pub position: Rank<CatalogPosition>,
}

/// A public channel the view asks a space to create and list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicChannelDraft {
    /// Minted by the view, so it can follow the channel before it exists.
    pub channel_id: Id<Channel>,
    pub name: ambre_core::Validated<ambre_core::Name>,
    pub topic: Option<Validated<ChannelTopic>>,
    /// Absent for a channel outside every category.
    pub category: Option<Id<Category>>,
    /// The rank in the catalogue: lower first, ties by identifier.
    pub position: Rank<CatalogPosition>,
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
    pub user_id: Id<User>,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub presence: Presence,
}

/// A private channel the view asks a space to create. It is listed nowhere,
/// so it has no topic, category nor rank in the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateChannelDraft {
    /// Minted by the view, so it can follow the channel before it exists.
    pub channel_id: Id<Channel>,
    pub name: ambre_core::Validated<ambre_core::Name>,
}

/// A direct message the view asks for: this user and `members`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMessageDraft {
    /// Minted by the view. It names the request, and the channel too when
    /// the relay creates one. An existing direct message keeps its own.
    pub channel_id: Id<Channel>,
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
    Opened { channel_id: Id<Channel> },
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
        category_id: Id<Category>,
        name: ambre_core::Validated<ambre_core::Name>,
        /// The rank among the categories: lower first, ties by identifier.
        position: Rank<CatalogPosition>,
    },
    /// Create a public channel, then list it in the catalogue with its key
    /// sealed under the space key (`MURMURE-037`).
    CreatePublicChannel {
        draft: PublicChannelDraft,
        /// The instant of the creation, stamped by the view.
        now: Instant<Authored>,
    },
    /// Create a private channel of the space, with this user as its owner and
    /// only member (`MURMURE-048`). The catalogue does not change
    /// (`MURMURE-050`).
    CreatePrivateChannel {
        draft: PrivateChannelDraft,
        /// The instant of the creation, stamped by the view.
        now: Instant<Authored>,
    },
    /// Join a public channel of the catalogue, with no other member online
    /// (`MURMURE-039`).
    JoinChannel { channel_id: Id<Channel> },
    /// Delete a channel of the space this user owns, with every document it
    /// groups. Once the relay answers, the client drops it, and a public one
    /// leaves the catalogue (`MURMURE-462`, `MURMURE-247`). A refusal keeps
    /// it whole (`MURMURE-463`).
    DeleteChannel { channel_id: Id<Channel> },
    /// Leave a channel of the space. Once the relay answers, the client drops
    /// it with its documents and its keys (`MURMURE-467`). A public one stays
    /// in the catalogue, and a private one's key is owed to the members who
    /// stay (`MURMURE-059`). A refusal keeps it whole.
    LeaveChannel { channel_id: Id<Channel> },
    /// Send this user's presence on the space, and turn offline the members
    /// whose presence expired. The view sends it once per refresh period
    /// (`ambre-spec-budgets.md section 8`): the Logic thread reads no clock.
    RefreshPresence {
        /// The view's clock. It never travels: it ages the members.
        now: Instant<Authored>,
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
        name: Option<ambre_core::Validated<ambre_core::Name>>,
        categories: Vec<CategoryView>,
        channels: Vec<CatalogueEntry>,
    },
    /// The members of the space, whole, after an open or any change: the
    /// online ones first, then the others, each group by display name.
    Members { members: Vec<MemberView> },
    /// The relay holds the new channel, and the catalogue lists it when it
    /// is public.
    ChannelCreated { channel_id: Id<Channel> },
    /// The user is now a member of the channel, which the view may open.
    Joined { channel_id: Id<Channel> },
    /// The relay deleted the channel, and this client holds nothing of it.
    ChannelDeleted { channel_id: Id<Channel> },
    /// The user left the channel, and this client holds nothing of it.
    ChannelLeft { channel_id: Id<Channel> },
    /// A creation, a join, a delete or a leave failed. Nothing changed on the
    /// relay.
    ChannelFailed {
        channel_id: Id<Channel>,
        message: String,
    },
    /// The space could not open. Nothing of it is held.
    OpenFailed { message: String },
    /// The user is no longer a member of the space: they left it, an owner
    /// removed them, or it was deleted. Nothing of it is held.
    Left,
}

/// Murmure's channel plane: the second type parameter of the envelopes its
/// page, the Logic runtime and the launcher send and receive (`BUS-027`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Murmure {}

impl ambre_bus::ChannelPlane for Murmure {
    type Command = MurmureCommand;
    type SpaceCommand = SpaceCommand;
    type DirectDraft = DirectMessageDraft;
    type Event = MurmureEvent;
    type SpaceEvent = SpaceEvent;
    type DirectEvent = DirectMessageEvent;
    type Matches = ChannelMatches;
}

// A view of another application carries no channel: its plane embeds into
// Murmure's through the uninhabited parts.
ambre_bus::embed_nothing!(
    MurmureCommand,
    SpaceCommand,
    DirectMessageDraft,
    MurmureEvent,
    SpaceEvent,
    DirectMessageEvent,
    ChannelMatches,
);

/// The UI to Logic envelope as a Murmure page sends it: no document command,
/// and Murmure's channel plane.
pub type LogicCommand = ambre_bus::LogicCommand<std::convert::Infallible, Murmure>;
/// The Logic to UI envelope as a Murmure page reads it.
pub type UiEvent = ambre_bus::UiEvent<std::convert::Infallible, Murmure>;

#[cfg(test)]
mod tests {
    use super::MurmureCommand;
    use ambre_bus::ChannelTopic;
    use ambre_core::Validated;
    use ambre_core::id::{Id, Message, Segment, User};

    /// A retry fetches again what the channel already holds: it writes no
    /// document, so a read-only member sends it too.
    #[test]
    fn a_retry_writes_no_document() {
        let retry = MurmureCommand::Retry {
            segment: Id::<Segment>::from_bytes([7; 16]),
        };
        assert!(!retry.is_mutating());
    }

    /// A topic, a draft, a file's name and a message body print their length
    /// in `Debug`. A log of an edit or of a search carries none of the text.
    #[test]
    fn a_text_of_a_conversation_prints_its_length_alone() {
        use super::{
            Attachment, ContentHash, Count, Deletion, MediaType, MessageAt, MessageDraft,
            MessageView, Octets, Revision,
        };
        use ambre_core::{Authored, Instant};
        let secret = "Q3 layoffs";
        let topic = Validated::<ChannelTopic>::parse(secret).expect("a topic");
        assert_eq!(format!("{topic:?}"), "10 chars");
        let draft = MessageDraft {
            text: secret.to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
        };
        let attachment = Attachment {
            media_ref: Validated::<ContentHash>::parse("a".repeat(64)).expect("an address"),
            name: secret.to_owned(),
            media_type: MediaType::new("text/plain".to_owned()),
            size: Count::<Octets>::new(1),
        };
        let view = MessageView {
            id: Id::<Message>::mint(),
            author: Id::<User>::mint(),
            ts: Instant::<Authored>::default(),
            text: secret.to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
            attachments: vec![attachment.clone()],
            reactions: Vec::new(),
            notice: None,
            edited: Revision::Original,
            deleted: Deletion::Live,
            thread: None,
            also_in_thread: None,
        };
        let edit = MurmureCommand::EditMessage {
            at: MessageAt::Segment(Id::<Segment>::from_bytes([7; 16])),
            message: Id::<Message>::mint(),
            draft: draft.clone(),
            now: Instant::<Authored>::default(),
        };
        for line in [
            format!("{draft:?}"),
            format!("{attachment:?}"),
            format!("{view:?}"),
            format!("{edit:?}"),
        ] {
            assert!(line.contains("10 chars"), "{line}");
            assert!(!line.contains(secret), "{line}");
        }
    }
}
