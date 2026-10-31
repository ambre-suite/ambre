//! The message list as the view draws it (the Murmure message list spec).
//!
//! What the Logic thread sends, folded into the rows a canvas paints. It
//! holds the messages of each channel and each thread, in order, and the
//! header and the words of each row. Pure, so the host tests hold it. The painter in
//! `paint` lays the rows out and draws them.

use ambre_core::index::{Axis, Count, IdxVec, Pos, Span};
use std::collections::{HashMap, HashSet, VecDeque};

use ambre_bus::{
    Attachment, Deletion, LogicCommand, MessageView, MurmureCommand, MurmureEvent, Notice,
    Reaction, Revision, TextStyle, Timestamp, UiEvent, Unreadable,
};
use ambre_core::id::{self, Channel, Document, Id, Message, User};
use ambre_core::{Byte, Char, Px, View};

use crate::MessageCount;

/// A row of a message list, counted from zero at the oldest. A key
/// outlives a load above, where a row does not.
#[derive(PartialEq, Eq)]
pub enum ListRow {}

impl Axis for ListRow {
    const NAME: &'static str = "ListRow";
}

/// The row `key` names among `keys`, `None` once it left the list.
pub fn row_of(keys: &IdxVec<ListRow, RowKey>, key: RowKey) -> Option<Pos<ListRow>> {
    keys.iter_enumerated()
        .find(|&(_, k)| *k == key)
        .map(|(row, _)| row)
}

/// Consecutive messages of one author inside this span share one header:
/// the grouping row of `ambre-spec-budgets.md section 8`.
pub const GROUP_SPAN: ambre_core::Millis = ambre_core::Millis::new(5 * 60 * 1000);

/// What a deleted message shows in place of its body (`MURMURE-185`).
pub const DELETED: &str = "This message was deleted.";

/// What follows the body of an edited message (`MURMURE-186`).
pub const EDITED: &str = "(edited)";

/// What the line above the first unread message reads (`MURMURE-184`).
pub const UNREAD_LINE: &str = "New messages";

/// What the line under a reply also sent to the channel reads
/// (`MURMURE-532`).
pub const FROM_THREAD: &str = "Replied in a thread";

/// What the control of a list read higher up reads (`MURMURE-505`).
pub const JUMP_TO_LATEST: &str = "Jump to latest";

/// What a screen reader hears when that control shows (`MURMURE-509`).
pub const JUMP_HINT: &str = "Newer messages below. Press End to jump to latest.";

/// The name a member shows before the view knows theirs.
pub const UNKNOWN_MEMBER: &str = "A member";

/// The messages of one channel or one thread, in the order the list shows
/// them: oldest at the top (`MURMURE-182`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageList {
    /// The segments, oldest first, each one's messages in its own order.
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    id: Id<id::Segment>,
    messages: Vec<MessageView>,
    /// The band that stands for the segment while it could not open
    /// (`MURMURE-218`). Its messages, once they arrive, take its place.
    band: Option<Band>,
}

impl Segment {
    /// The time the segment starts at, which places it in the list.
    fn start(&self) -> Option<Timestamp> {
        self.band
            .map(|b| b.from)
            .or_else(|| self.messages.first().map(|m| m.ts))
    }
}

/// A segment that could not open, shown as one row (`MURMURE-218`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    pub cause: Unreadable,
    pub from: Timestamp,
    /// The start of the next segment: `None` when none is listed after it.
    pub to: Option<Timestamp>,
}

/// What a band names when the keys of its segment never reached this
/// device (`MURMURE-218`).
pub const BAND_KEYS: &str = "the keys for these messages have not reached this device.";

/// What a band names when its segment opened under its key and failed
/// its check (`MURMURE-220`).
pub const BAND_DAMAGED: &str = "these messages could not be verified.";

/// The remedy a band names for a cause that asking again may clear.
pub const BAND_RETRY: &str = "Click, tap or press Enter to try again.";

impl Band {
    /// Whether asking again may clear its cause (`MURMURE-513`). A segment
    /// before the history boundary stays closed to this member.
    pub fn retries(&self) -> bool {
        self.cause != Unreadable::BeforeHistoryBoundary
    }
}

impl MessageList {
    /// Take the new or changed messages of `segment`, as
    /// [`MurmureEvent::Messages`] carries them.
    ///
    /// A message the list holds is replaced in place, and one it holds under
    /// another segment moves to `segment`. A new one comes after the messages
    /// of its segment. A segment the list does not hold takes its place by
    /// the time of its first message, so an older one lands above.
    pub fn absorb(&mut self, segment: Id<id::Segment>, messages: Vec<MessageView>) {
        let Some(first) = messages.first() else {
            return;
        };
        let first = first.ts;
        let incoming: HashSet<Id<Message>> = messages.iter().map(|m| m.id).collect();
        for other in self.segments.iter_mut().filter(|s| s.id != segment) {
            other.messages.retain(|m| !incoming.contains(&m.id));
        }
        self.segments
            .retain(|s| !s.messages.is_empty() || s.band.is_some());
        let at = self.place(segment, first);
        let Some(held) = self.segments.get_mut(at).map(|s| {
            s.band = None;
            &mut s.messages
        }) else {
            return;
        };
        let mut index: HashMap<Id<Message>, usize> =
            held.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
        for message in messages {
            match index.get(&message.id).and_then(|&i| held.get_mut(i)) {
                Some(slot) => *slot = message,
                None => {
                    index.insert(message.id, held.len());
                    held.push(message);
                }
            }
        }
    }

    /// Show `segment`, which could not open, as one band (`MURMURE-218`).
    /// A segment whose messages the list holds keeps them.
    pub fn unreadable(&mut self, segment: Id<id::Segment>, band: Band) {
        let at = self.place(segment, band.from);
        if let Some(held) = self.segments.get_mut(at)
            && held.messages.is_empty()
        {
            held.band = Some(band);
        }
    }

    /// Where `segment` sits: where the list holds it, or a new empty slot
    /// placed by `start`, so an older segment lands above.
    fn place(&mut self, segment: Id<id::Segment>, start: Timestamp) -> usize {
        if let Some(at) = self.segments.iter().position(|s| s.id == segment) {
            return at;
        }
        let at = self
            .segments
            .iter()
            .position(|s| s.start().is_some_and(|ts| ts > start))
            .unwrap_or(self.segments.len());
        self.segments.insert(
            at,
            Segment {
                id: segment,
                messages: Vec::new(),
                band: None,
            },
        );
        at
    }

    /// How many messages the list holds.
    pub fn len(&self) -> MessageCount {
        MessageCount::of_len(self.segments.iter().map(|s| s.messages.len()).sum())
    }

    /// The rows the list draws: a row per message and per band. A capacity,
    /// so a paint allocates once.
    fn row_capacity(&self) -> Count<ListRow> {
        Count::<ListRow>::from_len(
            self.segments
                .iter()
                .map(|s| s.messages.len() + usize::from(s.band.is_some()))
                .sum(),
        )
    }

    /// Whether the list holds no message.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// The newest message, at the bottom of the list.
    pub fn latest(&self) -> Option<&MessageView> {
        self.segments.iter().rev().find_map(|s| s.messages.last())
    }

    /// The oldest segment the list holds, read or shown as a band. The one
    /// before it is the next to load (`MURMURE-191`).
    pub fn oldest_segment(&self) -> Option<Id<id::Segment>> {
        self.segments.first().map(|s| s.id)
    }

    /// The segment that holds `message`, `None` when the list holds no such
    /// message.
    pub fn segment_of(&self, message: Id<Message>) -> Option<Id<id::Segment>> {
        self.segments
            .iter()
            .find(|s| s.messages.iter().any(|m| m.id == message))
            .map(|s| s.id)
    }

    /// What names each row, in the order of [`MessageList::rows`]. A key
    /// outlives a load above, where an index does not (`MURMURE-192`).
    pub fn keys(&self) -> IdxVec<ListRow, RowKey> {
        let mut keys = Vec::with_capacity(self.row_capacity().to_usize());
        for segment in &self.segments {
            if segment.band.is_some() {
                keys.push(RowKey::Band(segment.id));
            }
            keys.extend(segment.messages.iter().map(|m| RowKey::Message(m.id)));
        }
        IdxVec::<ListRow, _>::from(keys)
    }

    /// The rows, oldest first, each message marked with whether it opens a
    /// group under a header of its own (`MURMURE-183`). A band is one row,
    /// and the message after it opens a group.
    pub fn rows(&self) -> IdxVec<ListRow, Row<'_>> {
        let mut rows = Vec::with_capacity(self.row_capacity().to_usize());
        let mut previous: Option<&MessageView> = None;
        for segment in &self.segments {
            if let Some(band) = &segment.band {
                rows.push(Row::Band(band));
                previous = None;
            }
            for message in &segment.messages {
                let opens =
                    message.notice.is_none() && previous.is_none_or(|p| !grouped(p, message));
                let header = if opens { Header::Opens } else { Header::Joined };
                rows.push(Row::Message { message, header });
                previous = Some(message);
            }
        }
        IdxVec::<ListRow, _>::from(rows)
    }
}

/// Whether `next` joins the group of `previous`: one author, no notice
/// between, and inside the grouping span. A clock that runs back keeps the
/// group.
fn grouped(previous: &MessageView, next: &MessageView) -> bool {
    previous.author == next.author
        && previous.notice.is_none()
        && next.ts.since(previous.ts) <= GROUP_SPAN
}

ambre_core::define_flag! {
    /// Whether a message of the list opens its group under a header of its
    /// own (`MURMURE-183`).
    pub enum Header {
        /// It joins the group of the message above, under that header.
        Joined,
        /// A header opens its group: who wrote, and when.
        Opens,
    }

    /// The ink the body of a row draws in.
    pub enum BodyInk {
        /// The body is text a member wrote.
        Plain,
        /// The body is no text a member wrote: a deleted message, a notice
        /// or a band.
        Muted,
    }
}

/// One row of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    /// A message, and whether a header opens its group.
    Message {
        message: &'a MessageView,
        header: Header,
    },
    /// A segment that could not open.
    Band(&'a Band),
}

/// What names a row across loads: its message, or the segment of its band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKey {
    Message(Id<Message>),
    Band(Id<id::Segment>),
}

/// One style over the body a row shows, in bytes of that body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyMark {
    pub style: TextStyle,
    pub bytes: Span<Byte>,
}

/// The words of a header that opens a group: who wrote, and when.
///
/// `Debug` prints the length of the name alone: a member chose it.
#[derive(Clone, PartialEq, Eq)]
pub struct HeaderWords {
    /// The author's display name.
    pub author: String,
    /// The time the group's first message was sent, as the reader shows it.
    pub time: String,
}

impl std::fmt::Debug for HeaderWords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeaderWords")
            .field("author", &Count::<Byte>::from_len(self.author.len()))
            .field("time", &self.time)
            .finish()
    }
}

/// What a row shows, in words.
///
/// `Debug` prints the length of the body alone, and counts the marks and
/// the files: each is plaintext a member wrote or named.
#[derive(Clone, PartialEq, Eq)]
pub struct RowWords {
    /// The header, on a row that opens a group.
    pub header: Option<HeaderWords>,
    /// The body, or what stands in its place.
    pub body: String,
    /// Whether the body is text a member wrote, or muted.
    pub ink: BodyInk,
    /// The style marks of the body, over its bytes as shown (`MURMURE-565`).
    pub marks: Vec<BodyMark>,
    /// How many replies its thread holds, once it holds one. On a reply
    /// also sent to the channel, the line that names its thread
    /// (`MURMURE-532`).
    pub replies: Option<String>,
    /// The reactions under the body, one chip per emoji in emoji order
    /// (`MURMURE-137`). A notice and a band take none.
    pub reactions: Vec<Reaction>,
    /// The files under the body, one chip each, which a click fetches. A
    /// deleted message carries none.
    pub files: Vec<Attachment>,
}

impl std::fmt::Debug for RowWords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowWords")
            .field("header", &self.header)
            .field("body", &Count::<Byte>::from_len(self.body.len()))
            .field("ink", &self.ink)
            .field("marks", &self.marks.len())
            .field("replies", &self.replies)
            .field("reactions", &self.reactions.len())
            .field("files", &self.files.len())
            .finish()
    }
}

impl<'a> Row<'a> {
    /// The message of this row, `None` on a band.
    pub fn message(&self) -> Option<&'a MessageView> {
        match *self {
            Self::Message { message, .. } => Some(message),
            Self::Band(_) => None,
        }
    }

    /// The words of this row. `name_of` names a member, `time_of` writes a
    /// time in the reader's language and zone (`MURMURE-418`).
    pub fn words(
        &self,
        name_of: impl Fn(Id<User>) -> Option<ambre_core::DisplayName>,
        time_of: impl Fn(Timestamp) -> String,
    ) -> RowWords {
        let (message, header) = match *self {
            Self::Message { message, header } => (message, header),
            Self::Band(band) => return band_words(band, time_of),
        };
        let name = |user| {
            name_of(user)
                .map(ambre_core::DisplayName::into_string)
                .unwrap_or_else(|| UNKNOWN_MEMBER.to_owned())
        };
        let header = (header == Header::Opens).then(|| HeaderWords {
            author: name(message.author),
            time: time_of(message.ts),
        });
        let (body, ink, marks) = match message.notice {
            Some(Notice::MemberAdded { member }) => {
                let body = format!("{} joined the conversation", name(member));
                (body, BodyInk::Muted, Vec::new())
            }
            None if message.deleted == Deletion::Deleted => {
                (DELETED.to_owned(), BodyInk::Muted, Vec::new())
            }
            None => {
                let (mut body, bytes) = message.shown_at(&name_of);
                let marks = body_marks(message, &bytes);
                if message.edited == Revision::Edited {
                    body = format!("{body} {EDITED}");
                }
                (body, BodyInk::Plain, marks)
            }
        };
        let replies = message
            .thread
            .filter(|t| t.replies > ambre_bus::ReplyCount::ZERO)
            .map(|t| match t.replies.get() {
                1 => "1 reply".to_owned(),
                n => format!("{n} replies"),
            })
            .or_else(|| message.also_in_thread.map(|_| FROM_THREAD.to_owned()));
        let reactions = if message.notice.is_some() || message.deleted == Deletion::Deleted {
            Vec::new()
        } else {
            message.reactions.clone()
        };
        let files = if message.deleted == Deletion::Deleted {
            Vec::new()
        } else {
            message.attachments.clone()
        };
        RowWords {
            header,
            body,
            ink,
            marks,
            replies,
            reactions,
            files,
        }
    }

    /// What a screen reader hears of this row: the author, the time and the
    /// text of a message, under a header or not (`MURMURE-207`). A band says
    /// its words.
    pub fn announcement(
        &self,
        name_of: impl Fn(Id<User>) -> Option<ambre_core::DisplayName>,
        time_of: impl Fn(Timestamp) -> String,
    ) -> String {
        let Some(message) = self.message() else {
            return self.words(name_of, time_of).body;
        };
        let author = name_of(message.author)
            .map(ambre_core::DisplayName::into_string)
            .unwrap_or_else(|| UNKNOWN_MEMBER.to_owned());
        let time = time_of(message.ts);
        let words = self.words(&name_of, &time_of);
        let mut heard = match words.replies {
            Some(replies) => format!("{author}, {time}: {}. {replies}", words.body),
            None => format!("{author}, {time}: {}", words.body),
        };
        if !words.reactions.is_empty() {
            let chips: Vec<String> = words.reactions.iter().map(chip_label).collect();
            heard = format!("{heard}. Reactions: {}", chips.join(", "));
        }
        if !words.files.is_empty() {
            let files: Vec<String> = words.files.iter().map(file_label).collect();
            heard = format!("{heard}. Files: {}", files.join(", "));
        }
        heard
    }
}

/// What the chip of `reaction` reads: the emoji, and how many members chose
/// it.
pub fn chip_label(reaction: &Reaction) -> String {
    format!("{} {}", reaction.emoji.as_str(), reaction.users.len())
}

/// What the chip of a file reads: its name and its size.
pub fn file_label(file: &Attachment) -> String {
    format!("{} ({})", file.name, crate::uploads::size_label(file.size))
}

/// The words of a band: its span, its cause and its remedy, muted
/// (`MURMURE-218`). A segment before the history boundary is the start of
/// the channel, so it names no failure and no remedy (`MURMURE-219`).
fn band_words(band: &Band, time_of: impl Fn(Timestamp) -> String) -> RowWords {
    let span = match band.to {
        Some(to) => format!("Messages from {} to {}", time_of(band.from), time_of(to)),
        None => format!("Messages since {}", time_of(band.from)),
    };
    let body = match band.cause {
        Unreadable::KeyNotReceived => format!("{span}: {BAND_KEYS} {BAND_RETRY}"),
        Unreadable::Damaged => format!("{span}: {BAND_DAMAGED} {BAND_RETRY}"),
        Unreadable::BeforeHistoryBoundary => format!(
            "The history before {} was not shared with you.",
            time_of(band.to.unwrap_or(band.from))
        ),
    };
    RowWords {
        header: None,
        body,
        ink: BodyInk::Muted,
        marks: Vec::new(),
        replies: None,
        reactions: Vec::new(),
        files: Vec::new(),
    }
}

/// The style marks of `message` over the bytes of its shown body, `bytes`
/// giving where each char of its text starts there. A mark the shown body
/// leaves empty, such as one inside a renamed mention, is dropped.
fn body_marks(message: &MessageView, bytes: &IdxVec<Char, Pos<Byte>>) -> Vec<BodyMark> {
    let byte = |chars: Pos<Char>| bytes.get(chars).copied();
    crate::style::normalized(&message.styles)
        .into_iter()
        .filter_map(|mark| {
            let range = Span::<Byte>::new(byte(mark.range.start)?, byte(mark.range.end)?);
            (!range.is_empty()).then_some(BodyMark {
                style: mark.style,
                bytes: range,
            })
        })
        .collect()
}

/// The rows a viewport `height` tall shows when the list rests on its
/// latest message, each with the top of it, from the latest up.
///
/// Walked from the bottom, and `measure` is asked only for a row the walk
/// reaches: a row above the viewport is never laid out (`MURMURE-187`).
pub fn from_bottom(
    count: Count<ListRow>,
    height: Px<View>,
    mut measure: impl FnMut(Pos<ListRow>) -> Px<View>,
) -> Vec<(Pos<ListRow>, Px<View>)> {
    let mut shown = Vec::new();
    let mut bottom = height;
    for i in Span::<ListRow>::from_count(Pos::<ListRow>::ZERO, count)
        .into_iter()
        .rev()
    {
        if bottom <= Px::<View>::ZERO {
            break;
        }
        let top = bottom - measure(i);
        shown.push((i, top));
        bottom = top;
    }
    shown
}

/// How close to the bottom of the viewport the last row may end and still
/// rest the list on its latest message.
const FLUSH: Px<View> = Px::<View>::new(0.5);

ambre_core::define_flag! {
    /// Whether a row of the list is on screen.
    pub enum OnScreen {
        /// The viewport does not reach it.
        Hidden,
        /// The viewport shows it.
        Shown,
    }
}

/// The rows a viewport shows, top first, and where the list rests.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// Each row shown, top first: its index, its top and its height.
    pub rows: Vec<(Pos<ListRow>, Px<View>, Px<View>)>,
    /// Whether the oldest row is on screen.
    pub oldest: OnScreen,
    /// Whether the latest row is on screen.
    pub latest: OnScreen,
    /// The row the list rests on, and its top: the first row on screen.
    /// `None` when the list rests on its latest message.
    pub anchor: Option<(Pos<ListRow>, Px<View>)>,
}

/// The rows a viewport `height` tall shows with the row `anchor` names at
/// its top, or resting on the latest row with no anchor.
///
/// The list keeps a row in place, never a pixel offset, so rows loaded
/// above leave the screen still (`MURMURE-192`). A last row that ends
/// inside the viewport rests the list on its latest row. Each row is
/// measured once, and only the rows the walk reaches (`MURMURE-187`).
pub fn window(
    count: Count<ListRow>,
    height: Px<View>,
    anchor: Option<(Pos<ListRow>, Px<View>)>,
    mut measure: impl FnMut(Pos<ListRow>) -> Px<View>,
) -> Window {
    let mut known: HashMap<Pos<ListRow>, Px<View>> = HashMap::new();
    let mut tall = |i: Pos<ListRow>| *known.entry(i).or_insert_with(|| measure(i));
    let Some((at, top)) = anchor.filter(|&(at, _)| at < count.as_end()) else {
        return at_bottom(count, height, &mut tall);
    };
    let mut rows = VecDeque::new();
    let (mut next, mut y) = (at, top);
    if fill_down(&mut rows, (&mut next, &mut y), (count, height), &mut tall) {
        return at_bottom(count, height, &mut tall);
    }
    let (mut first, mut above) = (at, top);
    while above > Px::<View>::ZERO
        && let Some(previous) = first.prev()
    {
        first = previous;
        let h = tall(first);
        above -= h;
        rows.push_front((first, above, h));
    }
    if above > Px::<View>::ZERO {
        // The oldest row came down past the top: it rests there instead.
        for row in &mut rows {
            row.1 -= above;
        }
        y -= above;
        if fill_down(&mut rows, (&mut next, &mut y), (count, height), &mut tall) {
            return at_bottom(count, height, &mut tall);
        }
    }
    rows.retain(|&(_, top, h)| top + h > Px::<View>::ZERO && top < height);
    Window {
        oldest: OnScreen::from_bool(rows.front().is_some_and(|r| r.0 == Pos::<ListRow>::ZERO)),
        latest: OnScreen::from_bool(rows.back().is_some_and(|r| r.0.next() == count.as_end())),
        anchor: rows.front().map(|&(i, top, _)| (i, top)),
        rows: rows.into(),
    }
}

/// Lay rows out from row `next`, whose top is `y`, down to the bottom of the
/// viewport. Answers whether the last row ends inside it, which rests the
/// list on its latest row.
fn fill_down(
    rows: &mut VecDeque<(Pos<ListRow>, Px<View>, Px<View>)>,
    (next, y): (&mut Pos<ListRow>, &mut Px<View>),
    (count, height): (Count<ListRow>, Px<View>),
    tall: &mut impl FnMut(Pos<ListRow>) -> Px<View>,
) -> bool {
    while *next < count.as_end() && *y < height {
        let h = tall(*next);
        rows.push_back((*next, *y, h));
        *y += h;
        *next = next.next();
    }
    *next == count.as_end() && *y < height + FLUSH
}

/// The window of a list resting on its latest row.
fn at_bottom(
    count: Count<ListRow>,
    height: Px<View>,
    tall: &mut impl FnMut(Pos<ListRow>) -> Px<View>,
) -> Window {
    let mut shown = from_bottom(count, height, &mut *tall);
    shown.reverse();
    let rows: Vec<(Pos<ListRow>, Px<View>, Px<View>)> =
        shown.iter().map(|&(i, top)| (i, top, tall(i))).collect();
    Window {
        oldest: OnScreen::from_bool(rows.first().is_some_and(|r| r.0 == Pos::<ListRow>::ZERO)),
        latest: OnScreen::from_bool(!rows.is_empty()),
        anchor: None,
        rows,
    }
}

/// The message lists of a page: each open channel, and each thread its view
/// received.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::type_complexity,
    reason = "each `Id<_>` names its kind, and no alias may hide it"
)]
pub struct Conversations {
    channels: HashMap<Id<Channel>, MessageList>,
    /// Each thread by its channel and its root, with its document.
    threads: HashMap<(Id<Channel>, Id<Message>), (Id<Document>, MessageList)>,
    /// The first unread message of each channel, as the Logic thread last
    /// named it (`MURMURE-499`). Absent while its count is unknown.
    first_unread: HashMap<Id<Channel>, Option<Id<Message>>>,
    /// The other members typing in each channel, in its composer or in a
    /// thread's by its root, as the Logic thread last named them. A place
    /// where nobody types is absent, and so is a channel with none.
    typing: HashMap<Id<Channel>, HashMap<Option<Id<Message>>, Vec<Id<User>>>>,
}

impl Conversations {
    /// Fold one event of the Logic thread, and say whether a list changed.
    /// A channel the user left drops its list and its threads.
    pub fn absorb(&mut self, ev: UiEvent) -> bool {
        let UiEvent::Channel { channel_id, event } = ev else {
            return false;
        };
        match event {
            MurmureEvent::Messages { segment, messages } => {
                self.channels
                    .entry(channel_id)
                    .or_default()
                    .absorb(segment, messages);
                true
            }
            MurmureEvent::ThreadMessages {
                root,
                thread,
                messages,
            } => {
                let (_, list) = self
                    .threads
                    .entry((channel_id, root))
                    .or_insert_with(|| (thread, MessageList::default()));
                list.absorb(Id::<id::Segment>::from_document(thread), messages);
                true
            }
            MurmureEvent::Unreadable {
                segment,
                cause,
                from,
                to,
            } => {
                self.channels
                    .entry(channel_id)
                    .or_default()
                    .unreadable(segment, Band { cause, from, to });
                true
            }
            MurmureEvent::FirstUnread { message } => {
                self.first_unread.insert(channel_id, message) != Some(message)
            }
            MurmureEvent::Watchers {
                typing, in_threads, ..
            } => {
                let threads = in_threads
                    .into_iter()
                    .map(|(root, users)| (Some(root), users));
                let places: HashMap<Option<Id<Message>>, Vec<Id<User>>> =
                    std::iter::once((None, typing))
                        .chain(threads)
                        .filter(|(_, users)| !users.is_empty())
                        .collect();
                let held = self.typing.remove(&channel_id).unwrap_or_default();
                let changed = held != places;
                if !places.is_empty() {
                    self.typing.insert(channel_id, places);
                }
                changed
            }
            MurmureEvent::Left => {
                self.channels.remove(&channel_id);
                self.first_unread.remove(&channel_id);
                self.typing.remove(&channel_id);
                self.threads
                    .retain(|(channel, _), _| *channel != channel_id);
                true
            }
            _ => false,
        }
    }

    /// The list of an open channel.
    pub fn channel(&self, channel: Id<Channel>) -> Option<&MessageList> {
        self.channels.get(&channel)
    }

    /// The list of the thread of `root`: the same list a channel shows
    /// (`MURMURE-420`).
    pub fn thread(&self, channel: Id<Channel>, root: Id<Message>) -> Option<&MessageList> {
        self.threads.get(&(channel, root)).map(|(_, list)| list)
    }

    /// The document of the thread of `root`.
    pub fn thread_document(&self, channel: Id<Channel>, root: Id<Message>) -> Option<Id<Document>> {
        self.threads
            .get(&(channel, root))
            .map(|&(document, _)| document)
    }

    /// The first unread message of `channel`: `Some(None)` when all is
    /// read, `None` while the Logic thread named none yet.
    pub fn first_unread(&self, channel: Id<Channel>) -> Option<Option<Id<Message>>> {
        self.first_unread.get(&channel).copied()
    }

    /// The other members typing in `channel`, or in its thread under `root`
    /// (`MURMURE-537`). In the order the Logic thread gave, so the line never
    /// reorders between two draws.
    pub fn typing(&self, channel: Id<Channel>, root: Option<Id<Message>>) -> &[Id<User>] {
        self.typing
            .get(&channel)
            .and_then(|places| places.get(&root))
            .map_or(&[], Vec::as_slice)
    }
}

/// The line above the first unread message, held while its channel stays
/// on screen (`MURMURE-184`).
///
/// The list marks the channel read as soon as it shows the latest message.
/// So the line keeps what was unread when the channel came on screen.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnreadLine {
    /// The channel on screen, and the message the line sits above once
    /// known.
    #[expect(
        clippy::type_complexity,
        reason = "each `Id<_>` names its kind, and no alias may hide it"
    )]
    held: Option<(Id<Channel>, Option<Option<Id<Message>>>)>,
}

impl UnreadLine {
    /// The message the line sits above while `shown` is on screen. A new
    /// channel starts over, and its first known value then holds.
    pub fn above(
        &mut self,
        shown: Option<Id<Channel>>,
        talks: &Conversations,
    ) -> Option<Id<Message>> {
        let Some(channel) = shown else {
            self.held = None;
            return None;
        };
        let known = match self.held {
            Some((held, Some(known))) if held == channel => Some(known),
            _ => talks.first_unread(channel),
        };
        self.held = Some((channel, known));
        known.flatten()
    }

    /// The message the line sits above in `channel` at the last paint, with
    /// no lookup. `None` off that channel, or with no line.
    pub fn held(&self, channel: Id<Channel>) -> Option<Id<Message>> {
        match self.held {
            Some((held, Some(known))) if held == channel => known,
            _ => None,
        }
    }
}

/// The latest message the list drew of each channel and each thread, so a
/// read cursor moves once for each new message (`MURMURE-115`,
/// `MURMURE-497`).
#[derive(Debug, Clone, Default)]
pub struct ReadMarks {
    channels: HashMap<Id<Channel>, Id<Message>>,
    threads: HashMap<Id<Document>, Id<Message>>,
}

impl ReadMarks {
    /// The list drew the latest message of `channel`. The command that
    /// marks the channel read, unless it already did for this message.
    pub fn drawn(&mut self, channel: Id<Channel>, latest: Id<Message>) -> Option<LogicCommand> {
        (self.channels.insert(channel, latest) != Some(latest)).then_some(LogicCommand::Channel {
            channel_id: channel,
            command: MurmureCommand::MarkRead,
        })
    }

    /// The list drew the last reply of `thread`, in `channel`. The command
    /// that marks the thread read, unless it already did for this reply.
    pub fn thread_drawn(
        &mut self,
        channel: Id<Channel>,
        thread: Id<Document>,
        latest: Id<Message>,
    ) -> Option<LogicCommand> {
        (self.threads.insert(thread, latest) != Some(latest)).then_some(LogicCommand::Channel {
            channel_id: channel,
            command: MurmureCommand::MarkThreadRead { thread },
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ambre_bus::ThreadHead;

    use super::*;

    /// A message of `author`, written at `ts` milliseconds.
    pub(crate) fn message(author: Id<User>, ts: ambre_core::Timestamp, text: &str) -> MessageView {
        MessageView {
            id: Id::<Message>::mint(),
            author,
            ts,
            text: text.to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
            attachments: Vec::new(),
            reactions: Vec::new(),
            notice: None,
            edited: ambre_bus::Revision::Original,
            deleted: ambre_bus::Deletion::Live,
            thread: None,
            also_in_thread: None,
        }
    }

    /// The text of each row; a band reads as `#`.
    fn texts(list: &MessageList) -> Vec<&str> {
        list.rows()
            .iter()
            .map(|r| r.message().map_or("#", |m| m.text.as_str()))
            .collect()
    }

    fn headers(list: &MessageList) -> Vec<bool> {
        list.rows()
            .iter()
            .map(|r| {
                matches!(
                    r,
                    Row::Message {
                        header: Header::Opens,
                        ..
                    }
                )
            })
            .collect()
    }

    fn words(message: &MessageView, opens: bool) -> RowWords {
        let row = Row::Message {
            message,
            header: Header::from_bool(opens),
        };
        row.words(
            |_| ambre_core::DisplayName::new("Ada").ok(),
            |ts| format!("t{}", ts.get()),
        )
    }

    fn band(cause: Unreadable, from: u64, to: Option<u64>) -> Band {
        Band {
            cause,
            from: Timestamp::new(from),
            to: to.map(Timestamp::new),
        }
    }

    fn band_body(band: &Band) -> String {
        Row::Band(band)
            .words(|_| None, |ts| format!("t{}", ts.get()))
            .body
    }

    /// A segment that could not open shows as one muted row, placed by its
    /// span, naming the span, the cause and the remedy (`MURMURE-218`,
    /// `MURMURE-220`).
    #[test]
    fn an_unreadable_segment_shows_as_one_band_in_its_place() {
        let author = Id::<User>::mint();
        let (older, unread, newer) = (
            Id::<id::Segment>::mint(),
            Id::<id::Segment>::mint(),
            Id::<id::Segment>::mint(),
        );
        let mut list = MessageList::default();
        list.absorb(
            newer,
            vec![message(author, ambre_bus::Timestamp::new(30), "c")],
        );
        list.absorb(
            older,
            vec![message(author, ambre_bus::Timestamp::new(10), "a")],
        );
        list.unreadable(unread, band(Unreadable::KeyNotReceived, 20, Some(30)));
        assert_eq!(texts(&list), ["a", "#", "c"]);
        assert_eq!(
            headers(&list),
            [true, false, true],
            "a band breaks the group"
        );
        assert_eq!(list.len(), MessageCount::new(2));
        assert_eq!(list.latest().map(|m| m.text.as_str()), Some("c"));

        let keys = band(Unreadable::KeyNotReceived, 20, Some(30));
        let words = Row::Band(&keys).words(|_| None, |ts| format!("t{}", ts.get()));
        assert_eq!(
            words.body,
            format!("Messages from t20 to t30: {BAND_KEYS} {BAND_RETRY}")
        );
        assert_eq!(words.ink, BodyInk::Muted);
        assert!(words.header.is_none() && words.replies.is_none());
        assert_eq!(
            band_body(&band(Unreadable::Damaged, 20, None)),
            format!("Messages since t20: {BAND_DAMAGED} {BAND_RETRY}")
        );
        assert_eq!(
            band_body(&band(Unreadable::BeforeHistoryBoundary, 20, Some(30))),
            "The history before t30 was not shared with you."
        );
    }

    /// The messages of a segment take the place of its band, and a band
    /// never hides messages the list holds.
    #[test]
    fn messages_replace_the_band_of_their_segment() {
        let author = Id::<User>::mint();
        let (segment, read) = (Id::<id::Segment>::mint(), Id::<id::Segment>::mint());
        let mut list = MessageList::default();
        list.unreadable(segment, band(Unreadable::Damaged, 10, None));
        assert!(!list.is_empty());
        assert_eq!(list.latest(), None);
        list.unreadable(segment, band(Unreadable::KeyNotReceived, 10, None));
        assert_eq!(
            list.rows().as_slice(),
            [Row::Band(&band(Unreadable::KeyNotReceived, 10, None))],
            "a later cause replaces the earlier one"
        );
        list.absorb(
            segment,
            vec![message(author, ambre_bus::Timestamp::new(10), "a")],
        );
        assert_eq!(texts(&list), ["a"]);
        list.absorb(
            read,
            vec![message(author, ambre_bus::Timestamp::new(20), "b")],
        );
        list.unreadable(read, band(Unreadable::Damaged, 20, None));
        assert_eq!(texts(&list), ["a", "b"]);
    }

    /// The messages of the closing segment and of the active one read oldest
    /// at the top, whichever segment arrives first (`MURMURE-182`).
    #[test]
    fn segments_read_oldest_at_the_top_whatever_their_order_of_arrival() {
        let author = Id::<User>::mint();
        let (closing, active) = (Id::<id::Segment>::mint(), Id::<id::Segment>::mint());
        let mut list = MessageList::default();
        list.absorb(
            active,
            vec![
                message(author, ambre_bus::Timestamp::new(30), "c"),
                message(author, ambre_bus::Timestamp::new(40), "d"),
            ],
        );
        list.absorb(
            closing,
            vec![
                message(author, ambre_bus::Timestamp::new(10), "a"),
                message(author, ambre_bus::Timestamp::new(20), "b"),
            ],
        );
        assert_eq!(texts(&list), ["a", "b", "c", "d"]);
        assert_eq!(list.len(), MessageCount::new(4));
        assert_eq!(list.latest().map(|m| m.text.as_str()), Some("d"));
        list.absorb(
            active,
            vec![message(author, ambre_bus::Timestamp::new(50), "e")],
        );
        assert_eq!(texts(&list), ["a", "b", "c", "d", "e"]);
    }

    /// A delta replaces a message in place, and one sent under another
    /// segment moves there: the list never shows it twice (`MURMURE-182`).
    #[test]
    fn a_changed_message_takes_its_place_and_a_moved_one_leaves_its_old_segment() {
        let author = Id::<User>::mint();
        let (old, new) = (Id::<id::Segment>::mint(), Id::<id::Segment>::mint());
        let mut list = MessageList::default();
        let first = message(author, ambre_bus::Timestamp::new(10), "a");
        let second = message(author, ambre_bus::Timestamp::new(20), "b");
        list.absorb(old, vec![first.clone(), second.clone()]);
        let edited = MessageView {
            text: "a2".to_owned(),
            ..first.clone()
        };
        list.absorb(old, vec![edited]);
        assert_eq!(texts(&list), ["a2", "b"]);
        list.absorb(new, vec![first.clone(), second.clone()]);
        assert_eq!(texts(&list), ["a", "b"]);
        assert_eq!(list.segments.len(), 1);
        list.absorb(new, Vec::new());
        assert!(!list.is_empty());
        assert!(MessageList::default().is_empty());
    }

    /// One author's messages inside the grouping span share one header; a
    /// later one, another author or a notice opens a new group
    /// (`MURMURE-183`).
    #[test]
    fn one_author_inside_the_span_shares_one_header() {
        let (ada, bob) = (Id::<User>::mint(), Id::<User>::mint());
        let mut list = MessageList::default();
        let mut notice = message(ada, ambre_bus::Timestamp::new(2_000), "");
        notice.notice = Some(Notice::MemberAdded { member: bob });
        list.absorb(
            Id::<id::Segment>::mint(),
            vec![
                message(ada, ambre_bus::Timestamp::new(0), "a"),
                message(ada, ambre_bus::Timestamp::new(GROUP_SPAN.get()), "b"),
                message(
                    ada,
                    ambre_bus::Timestamp::new(2 * GROUP_SPAN.get() + 1),
                    "c",
                ),
                message(
                    bob,
                    ambre_bus::Timestamp::new(2 * GROUP_SPAN.get() + 2),
                    "d",
                ),
                notice,
                message(
                    bob,
                    ambre_bus::Timestamp::new(2 * GROUP_SPAN.get() + 3),
                    "e",
                ),
            ],
        );
        assert_eq!(headers(&list), [true, false, true, true, false, true]);
    }

    /// A clock that runs back keeps the group: the span saturates.
    #[test]
    fn a_clock_running_back_keeps_the_group() {
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        list.absorb(
            Id::<id::Segment>::mint(),
            vec![
                message(ada, ambre_bus::Timestamp::new(1_000), "a"),
                message(ada, ambre_bus::Timestamp::new(10), "b"),
            ],
        );
        assert_eq!(headers(&list), [true, false]);
    }

    /// The debug form of a row names the length of its body and of its
    /// author's name, never a char of either: both are plaintext.
    #[test]
    fn a_row_debugs_its_lengths_alone() {
        let message = message(
            Id::<User>::mint(),
            ambre_bus::Timestamp::new(7),
            "secret body",
        );
        let row = words(&message, true);
        let shown = format!("{row:?}");
        assert!(
            !shown.contains("secret") && !shown.contains("Ada"),
            "{shown}"
        );
        assert!(shown.contains("body: ByteCount(11)"), "{shown}");
        assert!(shown.contains("author: ByteCount(3)"), "{shown}");
    }

    /// A deleted message shows a muted line in place of its body
    /// (`MURMURE-185`).
    #[test]
    fn a_deleted_message_shows_a_muted_line() {
        let mut gone = message(Id::<User>::mint(), ambre_bus::Timestamp::new(7), "");
        gone.deleted = ambre_bus::Deletion::Deleted;
        let row = words(&gone, true);
        assert_eq!(row.body, DELETED);
        assert_eq!(row.ink, BodyInk::Muted);
        assert_eq!(
            row.header,
            Some(HeaderWords {
                author: "Ada".to_owned(),
                time: "t7".to_owned(),
            })
        );
    }

    /// An edited message ends with the marker; a plain one shows its body
    /// alone (`MURMURE-186`).
    #[test]
    fn an_edited_message_carries_the_marker() {
        let mut edited = message(Id::<User>::mint(), ambre_bus::Timestamp::new(0), "fixed");
        edited.edited = ambre_bus::Revision::Edited;
        assert_eq!(words(&edited, false).body, format!("fixed {EDITED}"));
        let plain = words(
            &message(Id::<User>::mint(), ambre_bus::Timestamp::new(0), "plain"),
            false,
        );
        assert_eq!(plain.body, "plain");
        assert_eq!(plain.ink, BodyInk::Plain);
        assert_eq!(plain.header, None);
    }

    /// A screen reader hears the author, the time and the text of a message,
    /// even one under the header of another. A band says its own words
    /// (`MURMURE-207`).
    #[test]
    fn a_message_is_announced_with_its_author_its_time_and_its_text() {
        let name_of = |_| ambre_core::DisplayName::new("Ada").ok();
        let time_of = |ts: Timestamp| format!("t{}", ts.get());
        let mut grouped = message(Id::<User>::mint(), ambre_bus::Timestamp::new(7), "hello");
        let row = Row::Message {
            message: &grouped,
            header: Header::Joined,
        };
        assert_eq!(row.announcement(name_of, time_of), "Ada, t7: hello");
        grouped.thread = Some(ThreadHead {
            document: Id::<Document>::mint(),
            replies: ambre_bus::ReplyCount::new(2),
            last_replier: None,
        });
        let row = Row::Message {
            message: &grouped,
            header: Header::Opens,
        };
        assert_eq!(
            row.announcement(name_of, time_of),
            "Ada, t7: hello. 2 replies"
        );
        let band = band(Unreadable::Damaged, 1, None);
        let row = Row::Band(&band);
        assert_eq!(
            row.announcement(name_of, time_of),
            row.words(name_of, time_of).body
        );
    }

    /// A notice names the member who joins, muted, and a member the view
    /// does not know reads under the stand-in name.
    #[test]
    fn a_notice_names_the_member_and_an_unknown_one_reads_as_a_member() {
        let mut notice = message(Id::<User>::mint(), ambre_bus::Timestamp::new(0), "");
        notice.notice = Some(Notice::MemberAdded {
            member: Id::<User>::mint(),
        });
        let row = words(&notice, false);
        assert_eq!(row.body, "Ada joined the conversation");
        assert_eq!(row.ink, BodyInk::Muted);
        let unknown = Row::Message {
            message: &notice,
            header: Header::Opens,
        }
        .words(|_| None, |_| String::new());
        assert_eq!(
            unknown.body,
            format!("{UNKNOWN_MEMBER} joined the conversation")
        );
        assert_eq!(
            unknown.header,
            Some(HeaderWords {
                author: UNKNOWN_MEMBER.to_owned(),
                time: String::new(),
            })
        );
    }

    /// A root counts its replies once its thread holds one.
    #[test]
    fn a_root_counts_its_replies() {
        let mut root = message(Id::<User>::mint(), ambre_bus::Timestamp::new(0), "root");
        let head = |replies| ThreadHead {
            document: Id::<Document>::mint(),
            replies: ambre_bus::ReplyCount::new(replies),
            last_replier: None,
        };
        root.thread = Some(head(0));
        assert_eq!(words(&root, false).replies, None);
        root.thread = Some(head(1));
        assert_eq!(words(&root, false).replies.as_deref(), Some("1 reply"));
        root.thread = Some(head(3));
        assert_eq!(words(&root, false).replies.as_deref(), Some("3 replies"));
    }

    /// A reply also sent to the channel reads a line that names its thread,
    /// and a screen reader hears it after the text (`MURMURE-532`).
    #[test]
    fn a_reply_sent_to_the_channel_too_names_its_thread() {
        let mut copy = message(Id::<User>::mint(), ambre_bus::Timestamp::new(0), "ok");
        assert_eq!(words(&copy, false).replies, None);
        copy.also_in_thread = Some(Id::<Document>::mint());
        assert_eq!(words(&copy, false).replies.as_deref(), Some(FROM_THREAD));
        let row = Row::Message {
            message: &copy,
            header: Header::Joined,
        };
        let heard = row.announcement(
            |_| ambre_core::DisplayName::new("Ada").ok(),
            |_| "t".to_owned(),
        );
        assert_eq!(heard, format!("Ada, t: ok. {FROM_THREAD}"));
    }

    /// Resting on the latest message, the walk measures only the rows the
    /// viewport reaches: five of ten thousand (`MURMURE-187`).
    #[test]
    fn the_walk_from_the_bottom_measures_only_the_rows_it_reaches() {
        let mut measured = Vec::new();
        let shown = from_bottom(Count::<ListRow>::new(10_000), px(100.0), |i| {
            measured.push(i.get());
            px(20.0)
        });
        assert_eq!(measured, [9_999, 9_998, 9_997, 9_996, 9_995]);
        assert_eq!(shown.first(), Some(&(Pos::<ListRow>::new(9_999), px(80.0))));
        assert_eq!(shown.last(), Some(&(Pos::<ListRow>::new(9_995), px(0.0))));
        assert!(from_bottom(Count::<ListRow>::ZERO, px(100.0), |_| px(1.0)).is_empty());
    }

    /// The indices and tops of a window's rows.
    fn tops(window: &Window) -> Vec<(u32, f32)> {
        window
            .rows
            .iter()
            .map(|&(i, top, _)| (i.get(), top.get()))
            .collect()
    }

    /// A length of the view.
    fn px(value: f32) -> Px<View> {
        Px::<View>::new(value)
    }

    /// A list resting on a row keeps that row where it is, and rows added
    /// below move nothing on screen (`MURMURE-190`).
    #[test]
    fn a_window_on_a_row_keeps_it_in_place_as_rows_arrive_below() {
        let rested = window(
            Count::<ListRow>::new(10),
            px(100.0),
            Some((Pos::<ListRow>::new(3), px(-5.0))),
            |_| px(20.0),
        );
        let expected = [
            (3, -5.0),
            (4, 15.0),
            (5, 35.0),
            (6, 55.0),
            (7, 75.0),
            (8, 95.0),
        ];
        assert_eq!(tops(&rested), expected);
        assert_eq!(rested.anchor, Some((Pos::<ListRow>::new(3), px(-5.0))));
        assert_eq!(
            (rested.oldest, rested.latest),
            (OnScreen::Hidden, OnScreen::Hidden)
        );
        let grown = window(
            Count::<ListRow>::new(12),
            px(100.0),
            Some((Pos::<ListRow>::new(3), px(-5.0))),
            |_| px(20.0),
        );
        assert_eq!(grown, rested);
    }

    /// A row pushed past the bottom rests the list on its latest row, so
    /// the next message scrolls into view (`MURMURE-189`).
    #[test]
    fn a_window_past_the_bottom_rests_on_the_latest_row() {
        let rested = window(
            Count::<ListRow>::new(10),
            px(100.0),
            Some((Pos::<ListRow>::new(8), px(0.0))),
            |_| px(20.0),
        );
        assert_eq!(rested.anchor, None);
        assert_eq!(tops(&rested).first(), Some(&(5, 0.0)));
        assert_eq!(tops(&rested).last(), Some(&(9, 80.0)));
        assert_eq!(rested.latest, OnScreen::Shown);
        let flush = window(
            Count::<ListRow>::new(10),
            px(100.0),
            Some((Pos::<ListRow>::new(5), px(0.0))),
            |_| px(20.0),
        );
        assert_eq!(
            flush.anchor, None,
            "a last row flush with the bottom rests there"
        );
        assert_eq!(
            window(
                Count::<ListRow>::new(10),
                px(100.0),
                Some((Pos::<ListRow>::new(10), px(0.0))),
                |_| px(20.0)
            )
            .anchor,
            None
        );
        let short = window(
            Count::<ListRow>::new(2),
            px(100.0),
            Some((Pos::<ListRow>::new(0), px(0.0))),
            |_| px(20.0),
        );
        assert_eq!(
            tops(&short),
            [(0, 60.0), (1, 80.0)],
            "a short list sits at the bottom"
        );
        assert_eq!(
            (short.oldest, short.latest),
            (OnScreen::Shown, OnScreen::Shown)
        );
    }

    /// The oldest row pulled down past the top rests at the top.
    #[test]
    fn a_window_past_the_top_rests_the_oldest_row_at_the_top() {
        let rested = window(
            Count::<ListRow>::new(10),
            px(100.0),
            Some((Pos::<ListRow>::new(1), px(30.0))),
            |_| px(20.0),
        );
        assert_eq!(rested.anchor, Some((Pos::<ListRow>::new(0), px(0.0))));
        assert_eq!(tops(&rested).last(), Some(&(4, 80.0)));
        assert_eq!(
            (rested.oldest, rested.latest),
            (OnScreen::Shown, OnScreen::Hidden)
        );
        let short = window(
            Count::<ListRow>::new(4),
            px(100.0),
            Some((Pos::<ListRow>::new(1), px(30.0))),
            |_| px(20.0),
        );
        assert_eq!(
            short.anchor, None,
            "a list shorter than the viewport rests on its latest"
        );
    }

    /// A window far up the list measures only the rows it shows, each once,
    /// and a flick that leaves rows above drops them (`MURMURE-187`).
    #[test]
    fn a_window_measures_only_the_rows_it_reaches_once_each() {
        let mut measured = Vec::new();
        let rested = window(
            Count::<ListRow>::new(10_000),
            px(100.0),
            Some((Pos::<ListRow>::new(5_000), px(0.0))),
            |i| {
                measured.push(i.get());
                px(20.0)
            },
        );
        assert_eq!(measured, [5_000, 5_001, 5_002, 5_003, 5_004]);
        assert_eq!(rested.rows.len(), 5);
        let flung = window(
            Count::<ListRow>::new(10_000),
            px(100.0),
            Some((Pos::<ListRow>::new(5_000), px(-45.0))),
            |_| px(20.0),
        );
        assert_eq!(flung.anchor, Some((Pos::<ListRow>::new(5_002), px(-5.0))));
    }

    /// Each row has a key, a band's being its segment, and the oldest
    /// segment is the first the list holds, read or not (`MURMURE-191`).
    #[test]
    fn each_row_has_a_key_and_the_oldest_segment_leads() {
        let author = Id::<User>::mint();
        let (older, newer) = (Id::<id::Segment>::mint(), Id::<id::Segment>::mint());
        let mut list = MessageList::default();
        assert_eq!(list.oldest_segment(), None);
        let later = message(author, ambre_bus::Timestamp::new(30), "c");
        list.absorb(newer, vec![later.clone()]);
        assert_eq!(list.oldest_segment(), Some(newer));
        list.unreadable(older, band(Unreadable::Damaged, 10, Some(30)));
        assert_eq!(list.oldest_segment(), Some(older));
        assert_eq!(
            list.keys().as_slice(),
            [RowKey::Band(older), RowKey::Message(later.id)]
        );
    }

    /// The band the Logic thread names lands in the list of its channel.
    #[test]
    fn an_unreadable_event_lands_as_a_band() {
        let channel = Id::<Channel>::mint();
        let mut talks = Conversations::default();
        let segment = Id::<id::Segment>::mint();
        assert!(talks.absorb(UiEvent::Channel {
            channel_id: channel,
            event: MurmureEvent::Unreadable {
                segment,
                cause: Unreadable::Damaged,
                from: Timestamp::new(5),
                to: None,
            },
        }));
        let list = talks.channel(channel).expect("a list");
        assert_eq!(
            list.rows().as_slice(),
            [Row::Band(&band(Unreadable::Damaged, 5, None))]
        );
    }

    /// A channel and a thread each fill their own list, and a channel left
    /// drops both (`MURMURE-420`).
    #[test]
    fn a_thread_fills_its_own_list_and_leaving_drops_the_channel() {
        let (channel, other) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let author = Id::<User>::mint();
        let root = message(author, ambre_bus::Timestamp::new(0), "root");
        let mut talks = Conversations::default();
        let document = Id::<Document>::mint();
        let ev = |channel_id, event| UiEvent::Channel { channel_id, event };
        assert!(talks.absorb(ev(
            channel,
            MurmureEvent::Messages {
                segment: Id::<id::Segment>::mint(),
                messages: vec![root.clone()],
            },
        )));
        assert!(talks.absorb(ev(
            channel,
            MurmureEvent::ThreadMessages {
                root: root.id,
                thread: document,
                messages: vec![message(author, ambre_bus::Timestamp::new(1), "reply")],
            },
        )));
        assert!(talks.absorb(ev(
            other,
            MurmureEvent::Messages {
                segment: Id::<id::Segment>::mint(),
                messages: vec![message(author, ambre_bus::Timestamp::new(2), "elsewhere")],
            },
        )));
        assert!(!talks.absorb(ev(
            channel,
            MurmureEvent::Watchers {
                watching: Vec::new(),
                typing: Vec::new(),
                in_threads: Vec::new(),
            }
        )));
        assert!(!talks.absorb(UiEvent::ChannelActivity {
            channels: Vec::new()
        }));
        let thread = talks.thread(channel, root.id).map(texts);
        assert_eq!(thread, Some(vec!["reply"]));
        assert_eq!(talks.thread_document(channel, root.id), Some(document));
        assert_eq!(talks.channel(channel).map(texts), Some(vec!["root"]));
        assert!(talks.absorb(ev(channel, MurmureEvent::Left)));
        assert!(talks.channel(channel).is_none());
        assert!(talks.thread(channel, root.id).is_none());
        assert!(talks.channel(other).is_some());
    }

    /// The list marks a channel read once per new latest message it draws
    /// (`MURMURE-115`).
    #[test]
    fn a_drawn_latest_message_marks_the_channel_read_once() {
        let channel = Id::<Channel>::mint();
        let (first, second) = (Id::<Message>::mint(), Id::<Message>::mint());
        let mut marks = ReadMarks::default();
        let read = |command: Option<LogicCommand>| {
            matches!(command, Some(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::MarkRead,
            }) if channel_id == channel)
        };
        assert!(read(marks.drawn(channel, first)));
        assert!(marks.drawn(channel, first).is_none());
        assert!(read(marks.drawn(channel, second)));
    }

    /// The list marks a thread read once per new last reply it draws, and
    /// apart from its channel (`MURMURE-497`).
    #[test]
    fn a_drawn_last_reply_marks_the_thread_read_once() {
        let (channel, thread) = (Id::<Channel>::mint(), Id::<Document>::mint());
        let (first, second) = (Id::<Message>::mint(), Id::<Message>::mint());
        let mut marks = ReadMarks::default();
        let read = |command: Option<LogicCommand>| {
            matches!(command, Some(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::MarkThreadRead { thread: t },
            }) if channel_id == channel && t == thread)
        };
        assert!(read(marks.thread_drawn(channel, thread, first)));
        assert!(marks.thread_drawn(channel, thread, first).is_none());
        assert!(
            marks.drawn(channel, first).is_some(),
            "the channel marks apart"
        );
        assert!(read(marks.thread_drawn(channel, thread, second)));
    }

    /// The line keeps the first unread message the channel had when it came
    /// on screen, and starts over on another channel (`MURMURE-184`).
    #[test]
    fn the_unread_line_holds_while_its_channel_stays_on_screen() {
        let (channel, other) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let first = Id::<Message>::mint();
        let mut talks = Conversations::default();
        let named = |talks: &mut Conversations, channel_id, message| {
            let event = MurmureEvent::FirstUnread { message };
            talks.absorb(UiEvent::Channel { channel_id, event })
        };
        let mut line = UnreadLine::default();
        assert_eq!(line.above(Some(channel), &talks), None, "nothing known yet");
        assert!(named(&mut talks, channel, Some(first)));
        assert!(!named(&mut talks, channel, Some(first)), "no change");
        assert_eq!(line.above(Some(channel), &talks), Some(first));

        assert!(named(&mut talks, channel, None));
        assert_eq!(
            line.above(Some(channel), &talks),
            Some(first),
            "the channel read on screen keeps its line"
        );
        assert_eq!(line.above(Some(other), &talks), None);
        assert_eq!(line.above(Some(channel), &talks), None, "back, all read");
        assert_eq!(line.above(None, &talks), None);
    }
}
