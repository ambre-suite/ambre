//! The message list as the view draws it (the Murmure message list spec).
//!
//! What the Logic thread sends, folded into the rows a canvas paints. It
//! holds the messages of each channel and each thread, in order, and the
//! header and the words of each row. Pure, so the host tests hold it. The painter in
//! `paint` lays the rows out and draws them.

use std::collections::{HashMap, HashSet};

use ambre_bus::{
    ChannelId, DocumentId, LogicCommand, MessageId, MessageView, MurmureCommand, MurmureEvent,
    Notice, SegmentId, Timestamp, UiEvent, UserId,
};

/// Consecutive messages of one author inside this span share one header:
/// the grouping row of `ambre-spec-budgets.md section 8`.
pub const GROUP_SPAN_MS: u64 = 5 * 60 * 1000;

/// What a deleted message shows in place of its body (`MURMURE-185`).
pub const DELETED: &str = "This message was deleted.";

/// What follows the body of an edited message (`MURMURE-186`).
pub const EDITED: &str = "(edited)";

/// What the line above the first unread message reads (`MURMURE-184`).
pub const UNREAD_LINE: &str = "New messages";

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
    id: SegmentId,
    messages: Vec<MessageView>,
}

impl MessageList {
    /// Take the new or changed messages of `segment`, as
    /// [`MurmureEvent::Messages`] carries them.
    ///
    /// A message the list holds is replaced in place, and one it holds under
    /// another segment moves to `segment`. A new one comes after the messages
    /// of its segment. A segment the list does not hold takes its place by
    /// the time of its first message, so an older one lands above.
    pub fn absorb(&mut self, segment: SegmentId, messages: Vec<MessageView>) {
        let Some(first) = messages.first() else {
            return;
        };
        let first = first.ts;
        let incoming: HashSet<MessageId> = messages.iter().map(|m| m.id).collect();
        for other in self.segments.iter_mut().filter(|s| s.id != segment) {
            other.messages.retain(|m| !incoming.contains(&m.id));
        }
        self.segments.retain(|s| !s.messages.is_empty());
        let at = match self.segments.iter().position(|s| s.id == segment) {
            Some(at) => at,
            None => {
                let at = self
                    .segments
                    .iter()
                    .position(|s| s.messages.first().is_some_and(|m| m.ts > first))
                    .unwrap_or(self.segments.len());
                self.segments.insert(
                    at,
                    Segment {
                        id: segment,
                        messages: Vec::new(),
                    },
                );
                at
            }
        };
        let Some(held) = self.segments.get_mut(at).map(|s| &mut s.messages) else {
            return;
        };
        let mut index: HashMap<MessageId, usize> =
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

    /// How many messages the list holds.
    pub fn len(&self) -> usize {
        self.segments.iter().map(|s| s.messages.len()).sum()
    }

    /// Whether the list holds no message.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// The newest message, at the bottom of the list.
    pub fn latest(&self) -> Option<&MessageView> {
        self.segments.last()?.messages.last()
    }

    /// The rows, oldest first, each marked with whether it opens a group
    /// under a header of its own (`MURMURE-183`).
    pub fn rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::with_capacity(self.len());
        let mut previous: Option<&MessageView> = None;
        for message in self.segments.iter().flat_map(|s| &s.messages) {
            let header = message.notice.is_none() && previous.is_none_or(|p| !grouped(p, message));
            rows.push(Row { message, header });
            previous = Some(message);
        }
        rows
    }
}

/// Whether `next` joins the group of `previous`: one author, no notice
/// between, and inside the grouping span. A clock that runs back keeps the
/// group.
fn grouped(previous: &MessageView, next: &MessageView) -> bool {
    previous.author == next.author
        && previous.notice.is_none()
        && next.ts.0.saturating_sub(previous.ts.0) <= GROUP_SPAN_MS
}

/// One row of the list: a message, and whether a header opens its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row<'a> {
    pub message: &'a MessageView,
    pub header: bool,
}

/// What a row shows, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowWords {
    /// The author's name and the time, on a row that opens a group.
    pub header: Option<(String, String)>,
    /// The body, or what stands in its place.
    pub body: String,
    /// The body is no text a member wrote: a deleted message or a notice.
    pub muted: bool,
    /// How many replies its thread holds, once it holds one.
    pub replies: Option<String>,
}

impl Row<'_> {
    /// The words of this row. `name_of` names a member, `time_of` writes a
    /// time in the reader's language and zone (`MURMURE-418`).
    pub fn words(
        &self,
        name_of: impl Fn(UserId) -> Option<String>,
        time_of: impl Fn(Timestamp) -> String,
    ) -> RowWords {
        let message = self.message;
        let name = |user| name_of(user).unwrap_or_else(|| UNKNOWN_MEMBER.to_owned());
        let header = self
            .header
            .then(|| (name(message.author), time_of(message.ts)));
        let (body, muted) = match message.notice {
            Some(Notice::MemberAdded { member }) => {
                (format!("{} joined the conversation", name(member)), true)
            }
            None if message.deleted => (DELETED.to_owned(), true),
            None if message.edited => (format!("{} {EDITED}", message.shown(&name_of)), false),
            None => (message.shown(&name_of), false),
        };
        let replies = message
            .thread
            .filter(|t| t.replies > 0)
            .map(|t| match t.replies {
                1 => "1 reply".to_owned(),
                n => format!("{n} replies"),
            });
        RowWords {
            header,
            body,
            muted,
            replies,
        }
    }
}

/// The rows a viewport `height` tall shows when the list rests on its
/// latest message, each with the top of it, from the latest up.
///
/// Walked from the bottom, and `measure` is asked only for a row the walk
/// reaches: a row above the viewport is never laid out (`MURMURE-187`).
pub fn from_bottom(
    count: usize,
    height: f32,
    mut measure: impl FnMut(usize) -> f32,
) -> Vec<(usize, f32)> {
    let mut shown = Vec::new();
    let mut bottom = height;
    for i in (0..count).rev() {
        if bottom <= 0.0 {
            break;
        }
        let top = bottom - measure(i);
        shown.push((i, top));
        bottom = top;
    }
    shown
}

/// The message lists of a page: each open channel, and each thread its view
/// received.
#[derive(Debug, Clone, Default)]
pub struct Conversations {
    channels: HashMap<ChannelId, MessageList>,
    /// Each thread by its channel and its root, with its document.
    threads: HashMap<(ChannelId, MessageId), (DocumentId, MessageList)>,
    /// The first unread message of each channel, as the Logic thread last
    /// named it (`MURMURE-499`). Absent while its count is unknown.
    first_unread: HashMap<ChannelId, Option<MessageId>>,
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
                list.absorb(SegmentId::from_document(thread), messages);
                true
            }
            MurmureEvent::FirstUnread { message } => {
                self.first_unread.insert(channel_id, message) != Some(message)
            }
            MurmureEvent::Left => {
                self.channels.remove(&channel_id);
                self.first_unread.remove(&channel_id);
                self.threads
                    .retain(|(channel, _), _| *channel != channel_id);
                true
            }
            _ => false,
        }
    }

    /// The list of an open channel.
    pub fn channel(&self, channel: ChannelId) -> Option<&MessageList> {
        self.channels.get(&channel)
    }

    /// The list of the thread of `root`: the same list a channel shows
    /// (`MURMURE-420`).
    pub fn thread(&self, channel: ChannelId, root: MessageId) -> Option<&MessageList> {
        self.threads.get(&(channel, root)).map(|(_, list)| list)
    }

    /// The document of the thread of `root`.
    pub fn thread_document(&self, channel: ChannelId, root: MessageId) -> Option<DocumentId> {
        self.threads
            .get(&(channel, root))
            .map(|&(document, _)| document)
    }

    /// The first unread message of `channel`: `Some(None)` when all is
    /// read, `None` while the Logic thread named none yet.
    pub fn first_unread(&self, channel: ChannelId) -> Option<Option<MessageId>> {
        self.first_unread.get(&channel).copied()
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
    held: Option<(ChannelId, Option<Option<MessageId>>)>,
}

impl UnreadLine {
    /// The message the line sits above while `shown` is on screen. A new
    /// channel starts over, and its first known value then holds.
    pub fn above(&mut self, shown: Option<ChannelId>, talks: &Conversations) -> Option<MessageId> {
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
}

/// The latest message the list drew of each channel and each thread, so a
/// read cursor moves once for each new message (`MURMURE-115`,
/// `MURMURE-497`).
#[derive(Debug, Clone, Default)]
pub struct ReadMarks {
    channels: HashMap<ChannelId, MessageId>,
    threads: HashMap<DocumentId, MessageId>,
}

impl ReadMarks {
    /// The list drew the latest message of `channel`. The command that
    /// marks the channel read, unless it already did for this message.
    pub fn drawn(&mut self, channel: ChannelId, latest: MessageId) -> Option<LogicCommand> {
        (self.channels.insert(channel, latest) != Some(latest)).then_some(LogicCommand::Channel {
            channel_id: channel,
            command: MurmureCommand::MarkRead,
        })
    }

    /// The list drew the last reply of `thread`, in `channel`. The command
    /// that marks the thread read, unless it already did for this reply.
    pub fn thread_drawn(
        &mut self,
        channel: ChannelId,
        thread: DocumentId,
        latest: MessageId,
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
    pub(crate) fn message(author: UserId, ts: u64, text: &str) -> MessageView {
        MessageView {
            id: MessageId::new(),
            author,
            ts: Timestamp(ts),
            text: text.to_owned(),
            mentions: Vec::new(),
            reactions: Vec::new(),
            notice: None,
            edited: false,
            deleted: false,
            thread: None,
            also_in_thread: None,
        }
    }

    fn texts(list: &MessageList) -> Vec<&str> {
        list.rows()
            .iter()
            .map(|r| r.message.text.as_str())
            .collect()
    }

    fn headers(list: &MessageList) -> Vec<bool> {
        list.rows().iter().map(|r| r.header).collect()
    }

    fn words(message: &MessageView, header: bool) -> RowWords {
        let row = Row { message, header };
        row.words(|_| Some("Ada".to_owned()), |ts| format!("t{}", ts.0))
    }

    /// The messages of the closing segment and of the active one read oldest
    /// at the top, whichever segment arrives first (`MURMURE-182`).
    #[test]
    fn segments_read_oldest_at_the_top_whatever_their_order_of_arrival() {
        let author = UserId::new();
        let (closing, active) = (SegmentId::new(), SegmentId::new());
        let mut list = MessageList::default();
        list.absorb(
            active,
            vec![message(author, 30, "c"), message(author, 40, "d")],
        );
        list.absorb(
            closing,
            vec![message(author, 10, "a"), message(author, 20, "b")],
        );
        assert_eq!(texts(&list), ["a", "b", "c", "d"]);
        assert_eq!(list.len(), 4);
        assert_eq!(list.latest().map(|m| m.text.as_str()), Some("d"));
        list.absorb(active, vec![message(author, 50, "e")]);
        assert_eq!(texts(&list), ["a", "b", "c", "d", "e"]);
    }

    /// A delta replaces a message in place, and one sent under another
    /// segment moves there: the list never shows it twice (`MURMURE-182`).
    #[test]
    fn a_changed_message_takes_its_place_and_a_moved_one_leaves_its_old_segment() {
        let author = UserId::new();
        let (old, new) = (SegmentId::new(), SegmentId::new());
        let mut list = MessageList::default();
        let first = message(author, 10, "a");
        let second = message(author, 20, "b");
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
        let (ada, bob) = (UserId::new(), UserId::new());
        let mut list = MessageList::default();
        let mut notice = message(ada, 2_000, "");
        notice.notice = Some(Notice::MemberAdded { member: bob });
        list.absorb(
            SegmentId::new(),
            vec![
                message(ada, 0, "a"),
                message(ada, GROUP_SPAN_MS, "b"),
                message(ada, 2 * GROUP_SPAN_MS + 1, "c"),
                message(bob, 2 * GROUP_SPAN_MS + 2, "d"),
                notice,
                message(bob, 2 * GROUP_SPAN_MS + 3, "e"),
            ],
        );
        assert_eq!(headers(&list), [true, false, true, true, false, true]);
    }

    /// A clock that runs back keeps the group: the span saturates.
    #[test]
    fn a_clock_running_back_keeps_the_group() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        list.absorb(
            SegmentId::new(),
            vec![message(ada, 1_000, "a"), message(ada, 10, "b")],
        );
        assert_eq!(headers(&list), [true, false]);
    }

    /// A deleted message shows a muted line in place of its body
    /// (`MURMURE-185`).
    #[test]
    fn a_deleted_message_shows_a_muted_line() {
        let mut gone = message(UserId::new(), 7, "");
        gone.deleted = true;
        let row = words(&gone, true);
        assert_eq!(row.body, DELETED);
        assert!(row.muted);
        assert_eq!(row.header, Some(("Ada".to_owned(), "t7".to_owned())));
    }

    /// An edited message ends with the marker; a plain one shows its body
    /// alone (`MURMURE-186`).
    #[test]
    fn an_edited_message_carries_the_marker() {
        let mut edited = message(UserId::new(), 0, "fixed");
        edited.edited = true;
        assert_eq!(words(&edited, false).body, format!("fixed {EDITED}"));
        let plain = words(&message(UserId::new(), 0, "plain"), false);
        assert_eq!(plain.body, "plain");
        assert!(!plain.muted);
        assert_eq!(plain.header, None);
    }

    /// A notice names the member who joins, muted, and a member the view
    /// does not know reads under the stand-in name.
    #[test]
    fn a_notice_names_the_member_and_an_unknown_one_reads_as_a_member() {
        let mut notice = message(UserId::new(), 0, "");
        notice.notice = Some(Notice::MemberAdded {
            member: UserId::new(),
        });
        let row = words(&notice, false);
        assert_eq!(row.body, "Ada joined the conversation");
        assert!(row.muted);
        let unknown = Row {
            message: &notice,
            header: true,
        }
        .words(|_| None, |_| String::new());
        assert_eq!(
            unknown.body,
            format!("{UNKNOWN_MEMBER} joined the conversation")
        );
        assert_eq!(
            unknown.header,
            Some((UNKNOWN_MEMBER.to_owned(), String::new()))
        );
    }

    /// A root counts its replies once its thread holds one.
    #[test]
    fn a_root_counts_its_replies() {
        let mut root = message(UserId::new(), 0, "root");
        let head = |replies| ThreadHead {
            document: DocumentId::new(),
            replies,
            last_replier: None,
        };
        root.thread = Some(head(0));
        assert_eq!(words(&root, false).replies, None);
        root.thread = Some(head(1));
        assert_eq!(words(&root, false).replies.as_deref(), Some("1 reply"));
        root.thread = Some(head(3));
        assert_eq!(words(&root, false).replies.as_deref(), Some("3 replies"));
    }

    /// Resting on the latest message, the walk measures only the rows the
    /// viewport reaches: five of ten thousand (`MURMURE-187`).
    #[test]
    fn the_walk_from_the_bottom_measures_only_the_rows_it_reaches() {
        let mut measured = Vec::new();
        let shown = from_bottom(10_000, 100.0, |i| {
            measured.push(i);
            20.0
        });
        assert_eq!(measured, [9_999, 9_998, 9_997, 9_996, 9_995]);
        assert_eq!(shown.first(), Some(&(9_999, 80.0)));
        assert_eq!(shown.last(), Some(&(9_995, 0.0)));
        assert!(from_bottom(0, 100.0, |_| 1.0).is_empty());
    }

    /// A channel and a thread each fill their own list, and a channel left
    /// drops both (`MURMURE-420`).
    #[test]
    fn a_thread_fills_its_own_list_and_leaving_drops_the_channel() {
        let (channel, other) = (ChannelId::new(), ChannelId::new());
        let author = UserId::new();
        let root = message(author, 0, "root");
        let mut talks = Conversations::default();
        let document = DocumentId::new();
        let ev = |channel_id, event| UiEvent::Channel { channel_id, event };
        assert!(talks.absorb(ev(
            channel,
            MurmureEvent::Messages {
                segment: SegmentId::new(),
                messages: vec![root.clone()],
            },
        )));
        assert!(talks.absorb(ev(
            channel,
            MurmureEvent::ThreadMessages {
                root: root.id,
                thread: document,
                messages: vec![message(author, 1, "reply")],
            },
        )));
        assert!(talks.absorb(ev(
            other,
            MurmureEvent::Messages {
                segment: SegmentId::new(),
                messages: vec![message(author, 2, "elsewhere")],
            },
        )));
        assert!(!talks.absorb(ev(
            channel,
            MurmureEvent::Watchers {
                watching: Vec::new(),
                typing: Vec::new(),
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
        let channel = ChannelId::new();
        let (first, second) = (MessageId::new(), MessageId::new());
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
        let (channel, thread) = (ChannelId::new(), DocumentId::new());
        let (first, second) = (MessageId::new(), MessageId::new());
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
        let (channel, other) = (ChannelId::new(), ChannelId::new());
        let first = MessageId::new();
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
