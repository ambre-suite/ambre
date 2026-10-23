//! The canvas view of a Murmure message region (the Murmure message list
//! spec).
//!
//! It paints the message list of what the pane shows, on any drawing
//! surface. It opens no document: the channel the sidebar selects binds what
//! it shows. The channel is the content canvas, and the thread is a canvas of
//! its own in the panel. Both read one set of lists, which the content canvas
//! fills from the page's queue.
//!
//! The page hands it the pane, the name of a member and the time as closures.
//! So the view holds no signal, and a host test drives it with plain values.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ambre_bus::conduit::{Receiver, Sender};
use ambre_bus::{
    Attachment, ByteSize, ChannelId, DocumentId, Emoji, LogicCommand, MessageAt, MessageId,
    MessageView, MurmureCommand, MurmureEvent, ReplyEcho, RequestId, RestEvent, SegmentId,
    TextStyle, Timestamp, UiEvent, UserId,
};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, PointerButton, RepaintPlan, TextCache,
    TextInput, Theme, ThemeControl,
};
use ambre_commands::{ClipboardWrite, Modifiers};
use ambre_core::{ViewPoint, ViewPx};
use ambre_draw::kinetic::{Kinetic, Release, ScrollStep};
use ambre_draw::{DrawSurface, FontContext, Point, Rect};

use crate::commands::{
    AskedAll, EDIT, EMOJI_KEY, FromThread, MessageAction, MessageTarget, PendingEdit,
    message_action,
};
use crate::composer::{Composer, Composition, DraftOf, EDITING_LINE, also_said, typing_line};
use crate::drafts::DraftShelf;
use crate::list::{Conversations, JUMP_HINT, MessageList, ReadMarks, Row, RowKey, UnreadLine};
use crate::markers::MarkBar;
use crate::mention_list::MentionList;
use crate::mention_list::OfferIndex;
use crate::paint::{Painted, Pictured, Shown, paint_jump, paint_list};
use crate::pictures::Pictures;
use crate::select::Selecting;
use crate::uploads::{PickedFile, Uploaded};
use crate::watchers::TICK_MS;
use crate::{MessageRef, MurmurePane, Region};

/// The id of the thread's canvas, in the panel. The shell names it, and the
/// host binds a cell to it while it is in the page.
pub const THREAD_CANVAS: ambre_core::CanvasId = ambre_core::CanvasId::new("murmure-thread-canvas");

/// What the page tells a region: the pane, a member's name, a time.
#[derive(Clone)]
pub struct PageReads {
    /// The pane the shell shows now.
    pub pane: Rc<dyn Fn() -> MurmurePane>,
    /// The name of a member, `None` when the page knows none.
    pub name_of: Rc<dyn Fn(UserId) -> Option<ambre_core::DisplayName>>,
    /// A time, in the reader's language and zone (`MURMURE-418`).
    pub time_of: Rc<dyn Fn(Timestamp) -> String>,
    /// The page's monotonic clock: the wheel and the pictures read it.
    pub now_ms: Rc<dyn Fn() -> ambre_core::PageTime>,
    /// The user's wall clock: it dates a message, an edit, a reaction and a
    /// typing sample. The page's clock restarts at every load, so it never
    /// dates anything a peer reads.
    pub wall_clock: Rc<dyn Fn() -> Timestamp>,
    /// The user asks for reduced motion: an animated picture holds its
    /// first frame (`MURMURE-130`).
    pub still: Rc<dyn Fn() -> bool>,
    /// Say a text to a screen reader, through the polite live region of the
    /// content (`CHROME-023`). It holds no element that takes the focus.
    pub announce: Rc<dyn Fn(&str)>,
    /// Hand the page the message a right-click or a long press fell on,
    /// `None` off any message. The page opens its actions; nothing is sent
    /// (`MURMURE-424`).
    pub act_on: Rc<dyn Fn(Option<MessageTarget>)>,
    /// Take the edit the member asked for from the actions of a message,
    /// which the channel's composer loads (`MURMURE-203`). Given `true`, it
    /// takes a reply alone, for the thread's composer (`MURMURE-526`).
    pub take_edit: Rc<dyn Fn(Region) -> Option<PendingEdit>>,
    /// Go to the pane before this one, as the back control of the header
    /// does: Escape in a thread's list closes it (`MURMURE-528`).
    pub back: Rc<dyn Fn()>,
    /// The signed-in member, whose last message the up arrow edits
    /// (`MURMURE-178`).
    pub me: Rc<dyn Fn() -> Option<UserId>>,
    /// The page's input pipe. A press in the composer gives it the keyboard
    /// focus, and its caret follows the composer's (`MURMURE-430`).
    pub pipe: TextInput,
    /// The members of a channel by name, as the Logic thread last listed
    /// them. The `@` list offers them (`MURMURE-551`).
    pub members_of: Rc<dyn Fn(ChannelId) -> Vec<(UserId, ambre_core::DisplayName)>>,
    /// Hand the page the `@` list to draw, or `None` to close it. The flag
    /// names the thread's region (`MURMURE-555`).
    pub show_mentions: Rc<dyn Fn(Region, Option<MentionList>)>,
    /// Take the entry a click picked in the `@` list. Given `true`, it takes
    /// a pick in the thread's list alone (`MURMURE-555`).
    pub take_pick: Rc<dyn Fn(Region) -> Option<OfferIndex>>,
    /// Hand the page the format bar to draw over a selection, or `None` to
    /// close it. The flag names the thread's region (`MURMURE-432`).
    pub show_bar: Rc<dyn Fn(Region, Option<MarkBar>)>,
    /// Take the style a press on the format bar asked for. Given `true`, it
    /// takes a press on the thread's bar alone (`MURMURE-432`).
    pub take_press: Rc<dyn Fn(Region) -> Option<TextStyle>>,
    /// Take the send its author cancelled at the `@channel` question, which
    /// the composer gets back. Given `true`, it takes a reply alone
    /// (`MURMURE-558`).
    pub take_returned: Rc<dyn Fn(Region) -> Option<AskedAll>>,
    /// Open the address of a link a click landed on, in a fresh tab with no
    /// opener (`MURMURE-567`).
    pub open_link: Rc<dyn Fn(&ambre_core::LinkUrl)>,
    /// The drafts the device keeps, which every region reads before a paint
    /// and writes after it (`MURMURE-210`).
    pub drafts: DraftShelf,
    /// Open the emoji picker on the composer of a region. The flag names
    /// the thread's region (`MURMURE-139`).
    pub ask_emoji: Rc<dyn Fn(Region)>,
    /// Take the emoji the picker picked for the composer. Given `true`, it
    /// takes a pick for the thread's composer alone (`MURMURE-139`).
    pub take_emoji: Rc<dyn Fn(Region) -> Option<ambre_bus::Emoji>>,
    /// Take the files the member dropped on the channel's canvas since the
    /// last frame. The channel's composer uploads them (`MURMURE-132`).
    pub take_files: Rc<dyn Fn() -> Vec<PickedFile>>,
    /// Hand the page a file a click on its chip fetched: an image opens at
    /// full size, any other file is saved.
    pub save_file: SaveFile,
    /// Warn the user, in a toast: an upload failed (`MURMURE-133`).
    pub warn: Rc<dyn Fn(&str)>,
}

/// What the page does with a file fetched, and its bytes.
pub type SaveFile = Rc<dyn Fn(&Attachment, Vec<u8>)>;

/// The id of the content canvas, which the channel's region paints. The
/// shell names it.
pub const CHANNEL_CANVAS: ambre_core::CanvasId = ambre_core::CanvasId::CONTENT;

/// How far down the viewport a message the pane names comes to rest.
const AT_FRACTION: f32 = 0.25;

/// How far a focused row may pass an edge of the viewport before the list
/// moves to show it: a rounding, not a scroll.
const REVEAL_SLACK_PX: f32 = 0.5;

/// Where the list of a region rests (`MURMURE-189`, `MURMURE-190`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Rest {
    /// On the latest row: a new message scrolls into view.
    Latest,
    /// With a row at a height, which rows loaded above or below leave in
    /// place (`MURMURE-192`).
    Row { key: RowKey, anchor_y: f32 },
}

/// The lists every region of the page reads, and a count of their changes.
#[derive(Default)]
struct Lists {
    conversations: Conversations,
    revision: u64,
}

/// The list a region shows: a channel, or the thread under one of its
/// messages. Two threads of one channel are two lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListOf {
    Channel(ChannelId),
    Thread(ChannelId, MessageId),
}

/// The canvas view of a Murmure message region.
pub struct MurmureView {
    /// The events of the Logic thread. The content canvas holds the page's
    /// queue; the thread's cell holds none and reads the lists it fills.
    from_logic: Option<Receiver<UiEvent>>,
    /// Where the read cursors move.
    to_logic: Option<Sender<LogicCommand>>,
    reads: PageReads,
    lists: Rc<RefCell<Lists>>,
    region: Region,
    marks: ReadMarks,
    unread_line: UnreadLine,
    /// The pane and the revision of the lists this region last painted.
    painted: (MurmurePane, u64),
    /// What the last paint did.
    last: Option<Painted>,
    theme_ctl: ThemeControl,
    theme: Theme,
    size: (f32, f32),
    /// Where the pointer last was over this region, in its own pixels. `None`
    /// before it came. The hover bar of a message reads it.
    hover: Option<Point>,
    /// Where the list rests.
    rest: Rest,
    /// The row [`Rest::Row`] names is not in the list yet: the list rests
    /// on its latest row until it comes.
    pending: bool,
    /// The first row on screen at the last paint, and its top.
    top_row: Option<(RowKey, f32)>,
    /// The way the last step went: below zero towards the oldest row.
    heading: f32,
    /// The scroll, through the model every application shares
    /// (`MURMURE-417`).
    kinetic: Kinetic,
    /// The oldest segment whose predecessor this region asked for.
    asked_older: Option<SegmentId>,
    /// Where the last paint drew the control back to the latest message,
    /// `None` when the latest message showed (`MURMURE-505`).
    jump: Option<Rect>,
    /// The row that holds the focus, `None` before Up or Down
    /// (`MURMURE-205`).
    focus: Option<RowKey>,
    /// The focus moved: the next paint brings its row into view.
    reveal: bool,
    /// The list and the latest message this region last heard, so a new one
    /// is announced once (`MURMURE-208`).
    heard: (Option<ListOf>, Option<(MessageId, Timestamp)>),
    /// The text selected, and the points of a drag the next paint resolves
    /// (`MURMURE-425`).
    selecting: Selecting,
    /// A mouse button holds a drag: `true` once it moved.
    drag: Option<bool>,
    /// The composer at the foot of the region, `MURMURE-199`. The thread's
    /// region holds one of its own (`MURMURE-523`).
    composer: Composer,
    /// The thread's composer while no thread shows. The channel's region
    /// holds it, so a draft outlives the thread's region (`MURMURE-530`).
    parked: Rc<RefCell<Option<Composer>>>,
    /// The files a click asked for, by the request that fetches each. Both
    /// regions share it, and the content canvas reads the answers.
    fetching: Rc<RefCell<HashMap<RequestId, Attachment>>>,
    /// The pictures the previews show. Both regions share it, and the
    /// content canvas takes the answers.
    pictures: Rc<RefCell<Pictures>>,
    /// When the next frame of a picture this region shows is due, `None`
    /// when none plays (`MURMURE-136`).
    next_frame: Option<ambre_core::PageTime>,
    /// The channel on screen this region last ticked, and when.
    ticked: Option<(ChannelId, ambre_core::PageTime)>,
    /// The `@` list this region last handed the page.
    listed: Option<MentionList>,
    /// The format bar this region last handed the page.
    barred: Option<MarkBar>,
    /// The version of the shelf the last plan read.
    shelf_seen: u64,
    dirty: bool,
}

impl Drop for MurmureView {
    /// The thread's region hands its composer back to the channel's, which
    /// gives it to the next thread's region (`MURMURE-530`).
    fn drop(&mut self) {
        if self.listed.is_some() {
            (self.reads.show_mentions)(self.region, None);
        }
        if self.barred.is_some() {
            (self.reads.show_bar)(self.region, None);
        }
        if self.region == Region::Thread {
            let mut composer = std::mem::take(&mut self.composer);
            composer.blur();
            *self.parked.borrow_mut() = Some(composer);
        }
    }
}

impl MurmureView {
    pub fn new(
        from_logic: Option<Receiver<UiEvent>>,
        to_logic: Option<Sender<LogicCommand>>,
        reads: PageReads,
        size: (f64, f64),
        theme_ctl: ThemeControl,
    ) -> Self {
        let theme = theme_ctl.get();
        Self {
            from_logic,
            to_logic,
            reads,
            lists: Rc::default(),
            region: Region::Channel,
            marks: ReadMarks::default(),
            unread_line: UnreadLine::default(),
            painted: (MurmurePane::default(), 0),
            last: None,
            theme_ctl,
            theme,
            size: (size.0 as f32, size.1 as f32),
            hover: None,
            rest: Rest::Latest,
            pending: false,
            top_row: None,
            heading: 0.0,
            kinetic: Kinetic::default(),
            asked_older: None,
            jump: None,
            focus: None,
            reveal: false,
            heard: (None, None),
            selecting: Selecting::default(),
            drag: None,
            composer: Composer::default(),
            parked: Rc::default(),
            fetching: Rc::default(),
            pictures: Rc::default(),
            next_frame: None,
            ticked: None,
            listed: None,
            barred: None,
            shelf_seen: 0,
            dirty: true,
        }
    }

    /// The view of the thread's region: the same theme and the same lists,
    /// and no queue of its own (`MURMURE-420`).
    pub fn region(&self, width: f64, height: f64) -> Self {
        let (to_logic, reads) = (self.to_logic.clone(), self.reads.clone());
        let theme_ctl = self.theme_ctl.clone();
        let mut view = Self::new(None, to_logic, reads, (width, height), theme_ctl);
        view.lists = Rc::clone(&self.lists);
        view.region = Region::Thread;
        view.parked = Rc::clone(&self.parked);
        view.fetching = Rc::clone(&self.fetching);
        view.pictures = Rc::clone(&self.pictures);
        if let Some(composer) = view.parked.borrow_mut().take() {
            view.composer = composer;
        }
        view
    }

    /// The size this region lays out in, in logical pixels.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// The height the list lays out in: the region, less the composer.
    fn list_height(&self) -> f32 {
        (self.size.1 - self.composer.band()).max(0.0)
    }

    /// The channel on screen in the channel's region, `None` in the thread's
    /// region or with no channel on screen.
    fn composer_channel(&self) -> Option<ChannelId> {
        let pane = (self.reads.pane)();
        pane.shown_channel()
            .filter(|_| self.region == Region::Channel)
    }

    /// The draft the composer of this region shows: the channel's, or the
    /// thread's in the panel. `None` with nothing on screen.
    fn writes_in(&self) -> Option<DraftOf> {
        match self.list_of((self.reads.pane)())? {
            ListOf::Channel(channel) => Some(DraftOf::Channel(channel)),
            ListOf::Thread(channel, root) => Some(DraftOf::Thread(channel, root)),
        }
    }

    /// Whether the height `y` falls in the composer's band.
    fn in_composer(&self, y: f64) -> bool {
        self.composer.band() > 0.0 && y as f32 >= self.list_height()
    }

    /// The composer, for the page and for tests.
    pub fn composer(&self) -> &Composer {
        &self.composer
    }

    /// Where the pointer last was over this region.
    pub fn hover(&self) -> Option<Point> {
        self.hover
    }

    /// What the last paint did, `None` before the first.
    pub fn last_painted(&self) -> Option<Painted> {
        self.last.clone()
    }

    /// Drain the inbound queue into the lists, so a full queue never stalls
    /// the Logic thread.
    pub fn update(&mut self) {
        let Some(from_logic) = &self.from_logic else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(ev) = from_logic.try_recv() {
            events.push(ev);
        }
        for ev in events {
            let Some(ev) = self.answer_file(ev) else {
                continue;
            };
            let mut lists = self.lists.borrow_mut();
            if lists.conversations.absorb(ev) {
                lists.revision += 1;
            }
        }
    }

    /// Take the answer to an upload or a fetch of a file, and give back any
    /// other event. An upload answered joins its draft (`MURMURE-132`). A
    /// failed one adds nothing, and the page told the user of the error
    /// (`MURMURE-133`).
    fn answer_file(&mut self, ev: UiEvent) -> Option<UiEvent> {
        match ev {
            UiEvent::Rest {
                id,
                evt:
                    RestEvent::MediaUploaded {
                        ref content_hash, ..
                    },
            } => match self.composer.uploads_mut().uploaded(id, content_hash) {
                None => return Some(ev),
                Some(Uploaded::Failed { name, .. }) => {
                    (self.reads.warn)(&format!("{name} could not be attached"));
                }
                Some(Uploaded::Ready(_)) => {}
            },
            UiEvent::Rest {
                id,
                evt: RestEvent::MediaDownloaded { data, .. },
            } if self.fetching.borrow().contains_key(&id) => {
                if let Some(file) = self.fetching.borrow_mut().remove(&id) {
                    (self.reads.save_file)(&file, data.into_bytes());
                }
            }
            UiEvent::ApiError { id: Some(id), .. } => {
                let upload = self.composer.uploads_mut().failed(id);
                let fetch = self.fetching.borrow_mut().remove(&id).is_some();
                if let Some(Uploaded::Failed { name, .. }) = &upload {
                    (self.reads.warn)(&format!("{name} could not be attached"));
                }
                if upload.is_none() && !fetch {
                    return Some(ev);
                }
            }
            UiEvent::Channel {
                event: MurmureEvent::Picture { media_ref, picture },
                ..
            } => {
                // Both regions repaint: either may show the picture.
                if self.pictures.borrow_mut().land(media_ref, picture) {
                    self.lists.borrow_mut().revision += 1;
                }
            }
            ev => return Some(ev),
        }
        self.dirty = true;
        None
    }

    /// Where the list rests: the row at the top and its top, `None` on the
    /// latest row.
    pub fn resting_on(&self) -> Option<(RowKey, f32)> {
        match self.rest {
            Rest::Row { key, anchor_y } if !self.pending => Some((key, anchor_y)),
            _ => None,
        }
    }

    /// Move the list by a step of the shared model: a positive step goes
    /// towards the latest row (`MURMURE-417`). The next paint holds it to
    /// the ends of the list.
    pub fn scroll_by(&mut self, step: ScrollStep) {
        let from = match self.rest {
            Rest::Row { key, anchor_y } if !self.pending => Some((key, anchor_y)),
            _ => self.top_row,
        };
        let Some((key, anchor_y)) = from else {
            return;
        };
        self.rest = Rest::Row {
            key,
            anchor_y: anchor_y - step.y(),
        };
        self.pending = false;
        self.heading = step.y();
        self.dirty = true;
    }

    /// Move the list by one viewport, towards the latest row for a positive
    /// `pages`. The model takes it as a wheel step of a page (`MURMURE-504`).
    fn page(&mut self, pages: f32) {
        let now_ms = (self.reads.now_ms)();
        let step = self
            .kinetic
            .wheel(0.0, pages * self.list_height(), true, now_ms);
        self.scroll_by(step);
    }

    /// Bring the list back to its latest message (`MURMURE-505`). The next
    /// paint shows it, and only then is it read (`MURMURE-506`).
    fn rest_at_latest(&mut self) {
        self.rest = Rest::Latest;
        self.pending = false;
        self.kinetic.stop();
        self.dirty = true;
    }

    /// Whether `at` falls on the control back to the latest message.
    fn on_jump(&self, at: Point) -> bool {
        self.jump.is_some_and(|rect| rect.contains(at))
    }

    /// The message under `(x, y)` at the last paint, as the target of its
    /// actions. `pressed` keeps the point, where a long press opens them.
    /// `None` on a band, between rows, or on the control to the latest.
    fn target_at(&self, x: f32, y: f32, pressed: bool) -> Option<MessageTarget> {
        let at = Point::new(x.into(), y.into());
        if self.on_jump(at) || self.in_composer(y.into()) {
            return None;
        }
        let i = self.last.as_ref()?.row_at(y)?;
        let mut target = self.target_of_row(i)?;
        target.pressed = pressed.then_some((x, y));
        Some(target)
    }

    /// The message of the row `i` of the list on screen, as the target of
    /// its actions. `None` on a band.
    fn target_of_row(&self, i: usize) -> Option<MessageTarget> {
        let pane = (self.reads.pane)();
        let lists = self.lists.borrow();
        let list = self.shown(&lists.conversations, pane)?;
        let message = list.rows().get(i)?.message()?;
        let (channel, at) = match (self.region, pane) {
            (Region::Thread, MurmurePane::Thread { channel, root }) => {
                (channel, MessageAt::Thread(root.message))
            }
            (Region::Channel, pane) => (
                pane.shown_channel()?,
                MessageAt::Segment(list.segment_of(message.id)?),
            ),
            (Region::Thread, _) => return None,
        };
        Some(self.target_of(channel, at, message))
    }

    /// A click on the chip `emoji` of the row `i`: the member's own reaction
    /// leaves (`MURMURE-096`), or theirs joins the others (`MURMURE-092`).
    /// Ask the relay for a file a click fell on. Its answer opens or saves
    /// it (`MURMURE-134`).
    fn fetch_file(&self, pane: MurmurePane, file: Attachment) {
        let channel_id = match self.list_of(pane) {
            Some(ListOf::Channel(channel) | ListOf::Thread(channel, _)) => channel,
            None => return,
        };
        let id = RequestId::next();
        let media_ref = file.media_ref.clone();
        self.fetching.borrow_mut().insert(id, file);
        self.send(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::FetchFile { id, media_ref },
        });
    }

    /// Ask the Logic thread for each picture the paint drew and the shelf
    /// lacks, and note when the next frame is due. Only the previews painted
    /// are on screen, so only they are fetched and play (`MURMURE-131`).
    fn show_pictures(&mut self, pane: MurmurePane, painted: &Painted) {
        self.next_frame = painted.next_frame;
        let channel_id = match self.list_of(pane) {
            Some(ListOf::Channel(channel) | ListOf::Thread(channel, _)) => channel,
            None => return,
        };
        let wanted: Vec<_> = {
            let mut shelf = self.pictures.borrow_mut();
            painted
                .pictures
                .iter()
                .filter(|media_ref| shelf.want(media_ref))
                .cloned()
                .collect()
        };
        for media_ref in wanted {
            self.send(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::ShowPicture { media_ref },
            });
        }
    }

    /// Ask the Logic thread to upload each file the page took for this
    /// region's draft. A thread's draft takes none (`MURMURE-128`).
    fn upload_files(&mut self) {
        if self.region != Region::Channel {
            return;
        }
        let files = (self.reads.take_files)();
        let Some(DraftOf::Channel(channel_id)) = self.writes_in() else {
            return;
        };
        for file in files {
            let id = RequestId::next();
            let (media_type, size) = (file.media_type(), ByteSize(file.data.len() as u64));
            let of = DraftOf::Channel(channel_id);
            if !self
                .composer
                .uploads_mut()
                .begin(id, of, file.name, media_type, size)
            {
                continue;
            }
            self.send(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::UploadFile {
                    id,
                    data: file.data,
                },
            });
            self.dirty = true;
        }
    }

    fn press_chip(&self, i: usize, emoji: Emoji) {
        let (Some(me), Some(target)) = ((self.reads.me)(), self.target_of_row(i)) else {
            return;
        };
        let mine = {
            let pane = (self.reads.pane)();
            let lists = self.lists.borrow();
            let message = self
                .shown(&lists.conversations, pane)
                .and_then(|list| list.rows().get(i)?.message());
            message.is_some_and(|m| {
                m.reactions
                    .iter()
                    .any(|r| r.emoji == emoji && r.users.contains(&me))
            })
        };
        let (at, message) = (target.at, target.message);
        let command = if mine {
            MurmureCommand::Unreact { at, message, emoji }
        } else {
            MurmureCommand::React {
                at,
                message,
                emoji,
                now: self.now(),
            }
        };
        self.send(LogicCommand::Channel {
            channel_id: target.channel,
            command,
        });
    }

    /// `message`, lying at `at` in `channel`, as the target of its actions.
    fn target_of(&self, channel: ChannelId, at: MessageAt, message: &MessageView) -> MessageTarget {
        MessageTarget {
            channel,
            at,
            message: message.id,
            author: message.author,
            text: message.text.clone(),
            mentions: message.mentions.clone(),
            styles: message.styles.clone(),
            notice: message.notice.is_some(),
            deleted: message.deleted,
            pressed: None,
            canvas: match self.region {
                Region::Channel => CHANNEL_CANVAS,
                Region::Thread => THREAD_CANVAS,
            },
            from_thread: message
                .also_in_thread
                .map(|thread| self.root_of(channel, thread)),
        }
    }

    /// Where the root of `thread` lies in the list of `channel`, so the
    /// reply action on its copy opens it (`MURMURE-533`).
    fn root_of(&self, channel: ChannelId, thread: DocumentId) -> FromThread {
        let lists = self.lists.borrow();
        let Some(list) = lists.conversations.channel(channel) else {
            return FromThread::Unloaded(thread);
        };
        let rows = list.rows();
        rows.iter()
            .filter_map(|row| row.message())
            .find(|message| message.thread.is_some_and(|t| t.document == thread))
            .and_then(|root| {
                let segment = list.segment_of(root.id)?;
                Some(FromThread::Root(MessageRef {
                    segment,
                    message: root.id,
                }))
            })
            .unwrap_or(FromThread::Unloaded(thread))
    }

    /// The edit of the latest message of the member in the channel or the
    /// thread `of`, that the edit action takes, `None` with none. Another
    /// member's message, a notice and a deleted message are passed over
    /// (`MURMURE-179`). In a thread, the latest reply (`MURMURE-527`).
    fn last_own_edit(&self, of: DraftOf) -> Option<PendingEdit> {
        let me = (self.reads.me)()?;
        let lists = self.lists.borrow();
        let talks = &lists.conversations;
        let list = match of {
            DraftOf::Channel(channel) => talks.channel(channel)?,
            DraftOf::Thread(channel, root) => talks.thread(channel, root)?,
        };
        let rows = list.rows();
        rows.iter()
            .rev()
            .filter_map(|row| row.message())
            .filter(|message| message.author == me)
            .find_map(|message| {
                let at = match of {
                    DraftOf::Channel(_) => MessageAt::Segment(list.segment_of(message.id)?),
                    DraftOf::Thread(_, root) => MessageAt::Thread(root),
                };
                let target = self.target_of(of.channel(), at, message);
                match message_action(EDIT, &target, Some(me)) {
                    Some(MessageAction::Edit(edit)) => Some(edit),
                    _ => None,
                }
            })
    }

    /// Mark the channel on screen read, as Escape in its list asks
    /// (`MURMURE-522`). `false` in the thread's region, which leaves the key.
    fn mark_shown_read(&self) -> bool {
        let Some(channel_id) = self.composer_channel() else {
            return false;
        };
        self.send(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::MarkRead,
        });
        true
    }

    /// Close the thread the thread's region shows, back to its channel at
    /// the root, and mark nothing read (`MURMURE-528`, `MURMURE-529`).
    /// Answers whether a thread closed.
    fn close_thread(&self) -> bool {
        let shown = matches!((self.reads.pane)(), MurmurePane::Thread { .. });
        if self.region != Region::Thread || !shown {
            return false;
        }
        (self.reads.back)();
        true
    }

    /// Bring the list to the message the unread line sits above, a quarter
    /// down (`MURMURE-175`). With no line, the list stays.
    fn rest_at_first_unread(&mut self) {
        let Some(message) = self
            .composer_channel()
            .and_then(|channel| self.unread_line.held(channel))
        else {
            return;
        };
        self.rest = Rest::Row {
            key: RowKey::Message(message),
            anchor_y: self.list_height() * AT_FRACTION,
        };
        self.pending = false;
        self.kinetic.stop();
        self.dirty = true;
    }

    /// Ask again for the segment of the band under `y` at the last paint, if
    /// its cause may clear (`MURMURE-513`). Answers whether one was asked.
    fn retry_at(&self, y: f32) -> bool {
        let Some(i) = self.last.as_ref().and_then(|painted| painted.row_at(y)) else {
            return false;
        };
        self.retry_row(|_| Some(i))
    }

    /// Ask again for the segment of the focused band (`MURMURE-513`).
    fn retry_focus(&self) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };
        self.retry_row(|keys| keys.iter().position(|key| *key == focus))
    }

    /// Ask again for the row `pick` finds among the keys of the list shown,
    /// when it is a band whose cause may clear. Only the channel region shows
    /// bands.
    fn retry_row(&self, pick: impl Fn(&[RowKey]) -> Option<usize>) -> bool {
        let pane = (self.reads.pane)();
        let Some(channel_id) = pane
            .shown_channel()
            .filter(|_| self.region == Region::Channel)
        else {
            return false;
        };
        let lists = self.lists.borrow();
        let Some(list) = self.shown(&lists.conversations, pane) else {
            return false;
        };
        let keys = list.keys();
        let Some(i) = pick(&keys) else {
            return false;
        };
        let (Some(RowKey::Band(segment)), Some(Row::Band(band))) =
            (keys.get(i).copied(), list.rows().get(i).copied())
        else {
            return false;
        };
        if !band.retries() {
            return false;
        }
        self.send(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::Retry { segment },
        });
        true
    }

    /// The focused message as the target of its actions, held at the foot
    /// of its row, where the menu opens below it (`MURMURE-511`). `None`
    /// with no focus on screen, or on a band.
    fn target_of_focus(&self) -> Option<MessageTarget> {
        let (top, bottom) = self.last.as_ref()?.focus?;
        let mut target = self.target_at(0.0, (top + bottom) / 2.0, false)?;
        target.pressed = Some((0.0, bottom));
        Some(target)
    }

    /// The row that holds the focus, `None` before Up or Down.
    pub fn focused(&self) -> Option<RowKey> {
        self.focus
    }

    /// Move the focus by `by` rows, towards the latest for a positive one,
    /// and announce the row it reaches (`MURMURE-205`, `MURMURE-207`). A
    /// focus off screen starts again from the last row on screen. No read
    /// cursor moves: only a paint that shows a message reads it
    /// (`MURMURE-206`).
    fn move_focus(&mut self, by: isize) {
        let lists = Rc::clone(&self.lists);
        let lists = lists.borrow();
        let Some(list) = self.shown(&lists.conversations, (self.reads.pane)()) else {
            return;
        };
        let keys = list.keys();
        // A focus just moved is on its way into view.
        let on_screen = self.reveal
            || self
                .last
                .as_ref()
                .is_some_and(|painted| painted.focus.is_some());
        let from = self
            .focus
            .and_then(|key| keys.iter().position(|k| *k == key));
        let at = match from {
            Some(i) if on_screen => i
                .checked_add_signed(by)
                .filter(|&next| next < keys.len())
                .unwrap_or(i),
            _ => match self.last.as_ref().and_then(|painted| painted.bottom) {
                Some(bottom) => bottom.min(keys.len().saturating_sub(1)),
                None => return,
            },
        };
        let (Some(&key), Some(row)) = (keys.get(at), list.rows().get(at).copied()) else {
            return;
        };
        self.focus = Some(key);
        self.reveal = true;
        self.dirty = true;
        (self.reads.announce)(&row.announcement(&*self.reads.name_of, &*self.reads.time_of));
    }

    /// Bring the focused row into view, once the paint placed it. A row off
    /// screen comes to rest at the top, and the next paint checks it again.
    fn reveal_focus(&mut self, painted: &Painted, at: Option<usize>, keys: &[RowKey]) {
        let Some(key) = at.and_then(|i| keys.get(i).copied()) else {
            self.reveal = false;
            return;
        };
        let Some((top, bottom)) = painted.focus else {
            self.rest = Rest::Row { key, anchor_y: 0.0 };
            self.pending = false;
            self.dirty = true;
            return;
        };
        self.reveal = false;
        let over = if top < -REVEAL_SLACK_PX {
            top
        } else if bottom > self.list_height() + REVEAL_SLACK_PX {
            (bottom - self.list_height()).min(top)
        } else {
            return;
        };
        let now_ms = (self.reads.now_ms)();
        let step = self.kinetic.wheel(0.0, over, true, now_ms);
        self.scroll_by(step);
    }

    /// A finger went down: it stops any motion left.
    pub fn touch_down(&mut self, x: f32, y: f32, now_ms: ambre_core::PageTime) {
        self.kinetic.press(x, y, now_ms);
    }

    /// A finger moved: past the tap slop the list follows it. Answers
    /// whether the list moved.
    pub fn touch_move(&mut self, x: f32, y: f32, now_ms: ambre_core::PageTime) -> bool {
        let Some(step) = self.kinetic.sample(x, y, now_ms) else {
            return false;
        };
        self.scroll_by(step);
        true
    }

    /// A finger lifted: a pan released with speed leaves a flick. Answers
    /// the point of a tap, save one the view takes. A tap on the control back
    /// to the latest message presses it, and one on a band asks for its
    /// segment again (`MURMURE-513`). A tap in the composer puts its caret
    /// there and gives it the keys.
    pub fn touch_up(&mut self, now_ms: ambre_core::PageTime) -> Option<(f32, f32)> {
        match self.kinetic.release(now_ms) {
            Release::Tap { x, y } if self.on_jump(Point::new(x.into(), y.into())) => {
                self.rest_at_latest();
                None
            }
            Release::Tap { y, .. } if self.composer.on_also(y) => {
                if let Some(of) = self.writes_in() {
                    self.composer.toggle_also(of);
                    self.dirty = true;
                }
                None
            }
            Release::Tap { x, y } if self.composer.file_at(x, y).is_some() => {
                if let (Some(of), Some(index)) = (self.writes_in(), self.composer.file_at(x, y)) {
                    self.composer.uploads_mut().remove(of, index);
                    self.dirty = true;
                }
                None
            }
            Release::Tap { x, y } if self.in_composer(y.into()) => {
                if let Some(of) = self.writes_in() {
                    self.composer.press(of, x, y);
                    self.composer.release();
                    self.reads.pipe.focus();
                    self.dirty = true;
                }
                None
            }
            Release::Tap { y, .. } if self.retry_at(y) => None,
            Release::Tap { x, y } => {
                self.selecting.click = Some((x, y));
                self.dirty = true;
                Some((x, y))
            }
            Release::Fling | Release::Rest => None,
        }
    }

    /// The browser took the finger: no tap and no flick.
    pub fn touch_cancel(&mut self) {
        self.kinetic.cancel();
    }

    /// Whether a flick or a glide is still moving the list.
    pub fn scrolling(&self) -> bool {
        self.kinetic.moving()
    }

    /// Whether a finger holds the list.
    pub fn finger_down(&self) -> bool {
        self.kinetic.pressed()
    }

    /// Whether a mouse button holds a drag across the text. The page keeps
    /// its moves past the canvas edge, where the list scrolls under it.
    pub fn drags_text(&self) -> bool {
        self.drag.is_some()
    }

    /// Another region of the page took the keys: they go back to the list,
    /// and the caret leaves the composer.
    pub fn leave_composer(&mut self) {
        if self.composer.focused() {
            self.composer.blur();
            self.dirty = true;
        }
    }

    /// The text a copy puts on the clipboard: each selected message under
    /// its author and its time, `None` with nothing selected
    /// (`MURMURE-426`).
    pub fn copy(&self) -> Option<String> {
        let selection = self.selecting.selection?;
        let lists = self.lists.borrow();
        let list = self.shown(&lists.conversations, (self.reads.pane)())?;
        selection.copied(list, &*self.reads.name_of, &*self.reads.time_of)
    }

    /// A drag past the top or the bottom edge scrolls the list by how far
    /// it passed.
    fn drag_past_edge(&mut self, y: f32) {
        let over = if y < 0.0 {
            y
        } else {
            (y - self.list_height()).max(0.0)
        };
        if over != 0.0 {
            let now_ms = (self.reads.now_ms)();
            let step = self.kinetic.wheel(0.0, over, true, now_ms);
            self.scroll_by(step);
        }
    }

    pub fn plan_repaint(&mut self, now_ms: ambre_core::PageTime, _hud_tick: bool) -> RepaintPlan {
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.dirty = true;
        }
        let shown = ((self.reads.pane)(), self.lists.borrow().revision);
        if shown.0 != self.painted.0 {
            self.rest_on(shown.0);
        }
        if shown != self.painted {
            self.painted = shown;
            self.dirty = true;
        }
        // A picture on screen reached its next frame (`MURMURE-136`).
        if self.next_frame.is_some_and(|at| now_ms >= at) {
            self.dirty = true;
        }
        // Rows read back after a reload, or a loss, change what the composer
        // shows: the next paint reads the shelf (`MURMURE-210`).
        let shelved = self.reads.drafts.version();
        if shelved != self.shelf_seen {
            self.shelf_seen = shelved;
            self.dirty = true;
        }
        if let Some(step) = self.kinetic.tick(now_ms)
            && step.y() != 0.0
        {
            self.scroll_by(step);
        }
        if let Some((x, y)) = self.kinetic.held(now_ms) {
            // A finger held still: the actions of its message open where it
            // went down (`MURMURE-423`).
            (self.reads.act_on)(self.target_at(x, y, true));
        }
        self.tick();
        // An edit asked from the actions of a message loads the channel's
        // composer, or the thread's for a reply, which takes the keys
        // (`MURMURE-203`, `MURMURE-526`).
        if let Some(edit) = (self.reads.take_edit)(self.region) {
            self.composer.start_edit(edit);
            self.reads.pipe.focus();
            self.dirty = true;
        }
        // A click in the `@` list picks its entry, and the keys stay in the
        // composer (`MURMURE-555`).
        if let Some(index) = (self.reads.take_pick)(self.region)
            && let Some(of) = self.writes_in()
        {
            self.composer.pick(of, index);
            self.typed();
            self.dirty = true;
        }
        // An emoji picked for the composer lands at its caret, as a paste
        // does, and the keys come back to it (`MURMURE-139`).
        if let Some(emoji) = (self.reads.take_emoji)(self.region)
            && let Some(of) = self.writes_in()
        {
            self.composer.text(of, emoji.as_str());
            self.reads.pipe.focus();
            self.typed();
            self.dirty = true;
        }
        // A press on the format bar toggles its style as its chord does, and
        // the keys stay in the composer (`MURMURE-432`).
        if let Some(style) = (self.reads.take_press)(self.region)
            && let Some(of) = self.writes_in()
        {
            self.composer.toggle(of, style);
            self.typed();
            self.dirty = true;
        }
        // A file dropped on the canvas uploads, and joins the draft once
        // the relay answers (`MURMURE-132`).
        self.upload_files();
        // A cancelled `@channel` gives its draft back, mentions and all
        // (`MURMURE-558`).
        if let Some(asked) = (self.reads.take_returned)(self.region) {
            let of = match &asked.held.reply {
                Some(reply) => DraftOf::Thread(asked.channel, reply.root),
                None => DraftOf::Channel(asked.channel),
            };
            let also = asked
                .held
                .reply
                .as_ref()
                .map_or(ReplyEcho::ThreadOnly, |r| r.also_in_channel);
            let mut held = asked.held;
            self.composer
                .uploads_mut()
                .restore(of, std::mem::take(&mut held.attachments));
            if self.composer.restore(of, held.draft, also) {
                self.reads.pipe.focus();
                self.dirty = true;
            }
        }
        let dirty = std::mem::take(&mut self.dirty);
        RepaintPlan {
            content: if dirty {
                ContentRepaint::Full
            } else {
                ContentRepaint::Reuse
            },
            content_strip: None,
            overlay: OverlayRepaint::Reuse,
            backdrop: self.theme.palette().content_bg,
        }
    }

    /// Paint the list this region shows, and move the read cursor once its
    /// latest message shows (`MURMURE-115`, `MURMURE-497`).
    pub fn paint_content<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        _strip: Option<Rect>,
    ) {
        let pane = (self.reads.pane)();
        let writes_in = self.writes_in();
        let (size, palette) = (self.size, self.theme.palette());
        // A draft kept on the device comes back on its paint, and the draft
        // painted is kept (`MURMURE-210`, `MURMURE-212`).
        self.composer.unshelve(writes_in, &self.reads.drafts);
        let sent = self.composer.flush(font_cx, writes_in, size, palette);
        self.composer.shelve(&self.reads.drafts);
        if let Some(sent) = sent {
            // The view stamps the time of the message or the edit
            // (`MURMURE-199`, `MURMURE-203`).
            let now = self.now();
            self.send(sent.command(now));
        }
        // The draft is drawn, so what the last key did is said
        // (`MURMURE-571` to `MURMURE-574`).
        if let Some(text) = self.composer.take_said() {
            (self.reads.announce)(&text);
        }
        let list_size = (self.size.0, self.list_height());
        let lists = Rc::clone(&self.lists);
        let lists = lists.borrow();
        let empty = MessageList::default();
        let list = self.shown(&lists.conversations, pane).unwrap_or(&empty);
        let on_screen = match self.region {
            Region::Channel => pane.shown_channel(),
            Region::Thread => None,
        };
        let unread = self.unread_line.above(on_screen, &lists.conversations);
        let keys = list.keys();
        let anchor = match self.rest {
            Rest::Row { key, anchor_y } => {
                keys.iter().position(|k| *k == key).map(|i| (i, anchor_y))
            }
            Rest::Latest => None,
        };
        self.pending = matches!(self.rest, Rest::Row { .. }) && anchor.is_none();
        let at_bottom = self.rest == Rest::Latest;
        let focus = self
            .focus
            .and_then(|key| keys.iter().position(|k| *k == key));
        if focus.is_none() {
            self.focus = None;
        }
        self.pictures.borrow_mut().begin_paint(self.region);
        let shelf = Rc::clone(&self.pictures);
        let shelf = shelf.borrow();
        let painted = paint_list(
            surface,
            font_cx,
            text_cache,
            Shown {
                list,
                unread,
                anchor,
                focus,
                selecting: self.selecting,
                me: (self.reads.me)(),
                pictures: Some(Pictured {
                    shelf: &shelf,
                    now_ms: (self.reads.now_ms)(),
                    still: (self.reads.still)(),
                }),
            },
            list_size,
            self.theme.palette(),
            &*self.reads.name_of,
            &*self.reads.time_of,
        );
        drop(shelf);
        self.show_pictures(pane, &painted);
        self.last = Some(painted.clone());
        if let Some(address) = &painted.link {
            (self.reads.open_link)(address);
        }
        if let Some((i, emoji)) = painted.chip.clone() {
            self.press_chip(i, emoji);
        }
        if let Some(file) = painted.file.clone() {
            self.fetch_file(pane, file);
        }
        // A selection moves no read cursor: only what the paint shows does
        // (`MURMURE-427`).
        self.selecting = Selecting {
            selection: painted.selection,
            ..Selecting::default()
        };
        self.settle(&painted, &keys);
        if self.reveal {
            self.reveal_focus(&painted, focus, &keys);
        }
        let higher_up = !painted.latest_shown && !list.is_empty();
        let palette = self.theme.palette();
        let shown_before = self.jump.is_some();
        self.jump = higher_up.then(|| paint_jump(surface, font_cx, text_cache, list_size, palette));
        // Who types where this region writes, by name: the channel, or the
        // thread (`MURMURE-517`, `MURMURE-537`).
        let names: Vec<Option<ambre_core::DisplayName>> = match writes_in {
            Some(DraftOf::Channel(channel)) => lists.conversations.typing(channel, None),
            Some(DraftOf::Thread(channel, root)) => lists.conversations.typing(channel, Some(root)),
            None => &[],
        }
        .iter()
        .map(|user| (self.reads.name_of)(*user))
        .collect();
        // An edit names itself in the strip, and how to leave it.
        let typing = match writes_in {
            Some(of) if self.composer.editing(of) => Some(EDITING_LINE.to_owned()),
            _ => typing_line(&names),
        };
        let caret = self.composer.paint(
            surface,
            font_cx,
            text_cache,
            writes_in,
            typing.as_deref(),
            self.size,
            palette,
        );
        if let Some((x, y, tall)) = caret {
            // The input method opens where the caret is (`MURMURE-430`).
            self.reads
                .pipe
                .set_caret(f64::from(x), f64::from(y), f64::from(tall.max(1.0)));
        }
        self.list_mentions(caret.map(|(x, y, _)| ViewPoint::new(ViewPx::new(x), ViewPx::new(y))));
        // The format bar hangs over a selection, and leaves with it.
        let bar = caret.zip(writes_in).and_then(|((x, y, _), of)| {
            self.composer.mark_bar(
                of,
                ViewPoint::new(ViewPx::new(x), ViewPx::new(y)),
                self.region,
            )
        });
        if bar != self.barred {
            self.barred.clone_from(&bar);
            (self.reads.show_bar)(self.region, bar);
        }
        if higher_up && !shown_before {
            // The control shows: End reaches it from the keyboard
            // (`MURMURE-509`).
            (self.reads.announce)(JUMP_HINT);
        }
        self.hear_latest(self.list_of(pane), list, at_bottom && painted.latest_shown);
        if self.region == Region::Channel
            && painted.oldest_shown
            && let Some((channel, oldest)) = on_screen.zip(list.oldest_segment())
            && self.asked_older.replace(oldest) != Some(oldest)
        {
            // The oldest row is on screen: the segment before it loads, once
            // per segment (`MURMURE-191`).
            self.send(LogicCommand::Channel {
                channel_id: channel,
                command: MurmureCommand::LoadOlder { segment: oldest },
            });
        }
        let Some(latest) = list.latest().map(|m| m.id).filter(|_| painted.latest_shown) else {
            return;
        };
        let command = match (self.region, pane) {
            (Region::Channel, pane) => pane
                .shown_channel()
                .and_then(|channel| self.marks.drawn(channel, latest)),
            (Region::Thread, MurmurePane::Thread { channel, root }) => lists
                .conversations
                .thread_document(channel, root.message)
                .and_then(|thread| self.marks.thread_drawn(channel, thread, latest)),
            (Region::Thread, _) => None,
        };
        if let Some(command) = command {
            self.send(command);
        }
    }

    /// Announce a message newer than the latest one this region heard, when
    /// the list was at the bottom (`MURMURE-208`). The first list a channel
    /// shows is not news, and the focus stays where it was (`MURMURE-209`).
    fn hear_latest(&mut self, of: Option<ListOf>, list: &MessageList, at_bottom: bool) {
        let latest = list.latest().map(|m| (m.id, m.ts));
        let (heard_in, heard) = std::mem::replace(&mut self.heard, (of, latest));
        let news = match (heard, latest) {
            (Some((old, old_ts)), Some((new, ts))) => old != new && ts >= old_ts,
            _ => false,
        };
        if !(news && at_bottom && heard_in == of) {
            return;
        }
        let rows = list.rows();
        let Some(row) = rows.iter().rev().find(|row| row.message().is_some()) else {
            return;
        };
        (self.reads.announce)(&row.announcement(&*self.reads.name_of, &*self.reads.time_of));
    }

    /// Offer the open `@` list the members of its channel, and hand the page
    /// what changed of it, hung from the `caret` (`MURMURE-551`,
    /// `MURMURE-555`). The entry chosen is said to a screen reader.
    fn list_mentions(&mut self, caret: Option<ViewPoint>) {
        if let Some(channel) = self.composer.asks_members_of() {
            let members = (self.reads.members_of)(channel);
            self.composer.offer(&members, (self.reads.me)());
        }
        let list = caret.and_then(|at| self.composer.mention_list(at, self.region));
        if list == self.listed {
            return;
        }
        let chosen = |list: &Option<MentionList>| {
            list.as_ref().and_then(|l| l.entries.get(l.chosen).cloned())
        };
        let said = chosen(&list);
        if let Some(text) = &said
            && said != chosen(&self.listed)
        {
            (self.reads.announce)(text);
        }
        self.listed.clone_from(&list);
        (self.reads.show_mentions)(self.region, list);
    }

    /// Hand `command` to the Logic thread. A full channel drops it, and the
    /// next paint that calls for it asks again.
    fn send(&self, command: LogicCommand) {
        if let Some(to_logic) = &self.to_logic
            && let Err(e) = to_logic.try_send(command)
        {
            tracing::warn!("Murmure view command dropped, Logic channel full ({e:?})");
        }
    }

    /// The wall clock, as the engine dates a sample or a message.
    fn now(&self) -> Timestamp {
        (self.reads.wall_clock)()
    }

    /// Tick the channel on screen once per tick period: the engine sends
    /// the user's watching sample and drops the samples past their expiry
    /// (`MURMURE-516`). A channel new on screen ticks at once.
    fn tick(&mut self) {
        let Some(channel_id) = self.composer_channel() else {
            self.ticked = None;
            return;
        };
        let now = (self.reads.now_ms)();
        let due = self
            .ticked
            .is_none_or(|(ticked, at)| ticked != channel_id || now - at >= TICK_MS.as_page_ms());
        if due {
            self.ticked = Some((channel_id, now));
            let now = self.now();
            self.send(LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::Tick { now },
            });
        }
    }

    /// A key changed the draft: the engine refreshes the user's typing
    /// indicator, once per typing period (`MURMURE-515`). A thread's draft
    /// names its root (`MURMURE-536`).
    fn typed(&mut self) {
        if !self.composer.take_typed() {
            return;
        }
        let (channel_id, thread) = match self.writes_in() {
            Some(DraftOf::Channel(channel)) => (channel, None),
            Some(DraftOf::Thread(channel, root)) => (channel, Some(root)),
            None => return,
        };
        let now = self.now();
        self.send(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::Typing { now, thread },
        });
    }

    /// Rest the list where the pane shows it: at the message it names, a
    /// quarter down, or on the latest row (`MURMURE-150`).
    fn rest_on(&mut self, pane: MurmurePane) {
        self.rest = match (self.region, pane) {
            (Region::Channel, MurmurePane::Channel { at: Some(at), .. }) => Rest::Row {
                key: RowKey::Message(at.message),
                anchor_y: self.list_height() * AT_FRACTION,
            },
            _ => Rest::Latest,
        };
        self.top_row = None;
        self.asked_older = None;
        self.focus = None;
        self.reveal = false;
        self.selecting = Selecting::default();
        self.drag = None;
        self.kinetic.stop();
    }

    /// Keep where the paint left the list, and stop a motion that met an
    /// end of it.
    fn settle(&mut self, painted: &Painted, keys: &[RowKey]) {
        let key_of = |(i, y): (usize, f32)| keys.get(i).map(|&key| (key, y));
        self.top_row = painted.top.and_then(key_of);
        if !self.pending {
            self.rest = painted
                .rest
                .and_then(key_of)
                .map_or(Rest::Latest, |(key, anchor_y)| Rest::Row { key, anchor_y });
        }
        let at_latest = painted.rest.is_none();
        let at_oldest = painted.rest.is_some_and(|(i, y)| i == 0 && y >= 0.0);
        let edge = if self.heading > 0.0 {
            at_latest
        } else {
            at_oldest || (at_latest && painted.oldest_shown)
        };
        if self.heading != 0.0 && edge {
            self.kinetic.stop();
        }
        self.heading = 0.0;
    }

    /// Which list the pane shows in this region, `None` when it shows none.
    fn list_of(&self, pane: MurmurePane) -> Option<ListOf> {
        match (self.region, pane) {
            (Region::Channel, pane) => pane.shown_channel().map(ListOf::Channel),
            (Region::Thread, MurmurePane::Thread { channel, root }) => {
                Some(ListOf::Thread(channel, root.message))
            }
            (Region::Thread, _) => None,
        }
    }

    /// The list of what the pane shows in this region: the channel's in the
    /// content canvas, the thread's in the panel.
    fn shown<'a>(&self, talks: &'a Conversations, pane: MurmurePane) -> Option<&'a MessageList> {
        match (self.region, pane) {
            (Region::Channel, pane) => talks.channel(pane.shown_channel()?),
            (Region::Thread, MurmurePane::Thread { channel, root }) => {
                talks.thread(channel, root.message)
            }
            (Region::Thread, _) => None,
        }
    }

    pub fn paint_overlay<S: DrawSurface>(
        &mut self,
        _surface: &mut S,
        _font_cx: &mut FontContext,
        _text_cache: &mut TextCache,
        _stats: &FrameStats,
    ) {
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width as f32, height as f32);
        self.dirty = true;
    }

    /// The window is as wide as a desktop, or no longer is. The box under a
    /// thread's composer names its chord only then (`MURMURE-544`).
    pub fn set_desktop(&mut self, desktop: bool) {
        if self.composer.show_chords(desktop) {
            self.dirty = true;
        }
    }

    /// The region follows the pointer, and the wheel scrolls it. A trackpad
    /// glide that pauses coasts on; a notch never does (`KINETIC-002`). The
    /// pinch chord is the page's zoom, so the list leaves it. Page Up, Page
    /// Down and End jump through the list. Up and Down move the focus, and a
    /// key with a modifier is the browser's (`MURMURE-504`, `MURMURE-505`,
    /// `MURMURE-205`). The ContextMenu key and Shift+F10 open the actions of
    /// the focused message (`MURMURE-511`).
    ///
    /// Shift+Page Up goes to the unread line (`MURMURE-175`), and Escape
    /// marks the channel read (`MURMURE-522`).
    ///
    /// A click on a band, or Enter on a focused one, asks for its segment
    /// again (`MURMURE-513`).
    ///
    /// A drag of the main button selects text, and leaves the press to the
    /// page, so the canvas takes the focus (`MURMURE-425`).
    ///
    /// The composer takes the text of the pipe, and a press in its band. Once
    /// it holds the focus, it takes the keys it edits with. Page Up and Page
    /// Down still move the list.
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        if let Some(consumed) = self.composer_input(ev) {
            self.typed();
            self.dirty = true;
            return consumed;
        }
        match *ev {
            InputEvent::PointerDown { x, y, button, .. } => {
                let at = Point::new(x, y);
                self.hover = Some(at);
                // A press in the list gives the keys back to it.
                if self.composer.focused() {
                    self.composer.blur();
                    self.dirty = true;
                }
                if button == PointerButton::Secondary {
                    // The page's menu event follows, and opens the actions of
                    // the message found here (`MURMURE-422`).
                    (self.reads.act_on)(self.target_at(x as f32, y as f32, false));
                } else if button == PointerButton::Primary && !self.on_jump(at) {
                    self.selecting = Selecting {
                        press: Some((x as f32, y as f32)),
                        ..Selecting::default()
                    };
                    self.drag = Some(false);
                    self.dirty = true;
                }
            }
            InputEvent::PointerMove { x, y } => {
                self.hover = Some(Point::new(x, y));
                if let Some(moved) = self.drag.as_mut() {
                    *moved = true;
                    self.selecting.drag = Some((x as f32, y as f32));
                    self.dirty = true;
                    self.drag_past_edge(y as f32);
                }
            }
            InputEvent::PointerUp { x, y, .. } => {
                let at = Point::new(x, y);
                self.hover = Some(at);
                let drag = self.drag.take();
                if drag == Some(true) {
                    self.selecting.drag = Some((x as f32, y as f32));
                    self.dirty = true;
                    return false;
                }
                if self.on_jump(at) {
                    self.rest_at_latest();
                    return true;
                }
                // A click of the main button on a band asks for its segment
                // again (`MURMURE-513`).
                if drag == Some(false) && self.retry_at(y as f32) {
                    return true;
                }
                // A click that moved nothing may land on a link, which the
                // next paint finds under it (`MURMURE-567`).
                if drag == Some(false) {
                    self.selecting.click = Some((x as f32, y as f32));
                    self.dirty = true;
                }
            }
            InputEvent::Key {
                ref key,
                shift,
                ctrl: false,
                alt: false,
                meta: false,
            } => {
                match key.as_str() {
                    "ContextMenu" => (self.reads.act_on)(self.target_of_focus()),
                    "F10" if shift => (self.reads.act_on)(self.target_of_focus()),
                    "PageUp" if shift => self.rest_at_first_unread(),
                    "PageDown" => self.page(1.0),
                    "PageUp" => self.page(-1.0),
                    "End" => self.rest_at_latest(),
                    "ArrowDown" => self.move_focus(1),
                    "ArrowUp" => self.move_focus(-1),
                    "Enter" if self.retry_focus() => {}
                    "Escape" if self.mark_shown_read() || self.close_thread() => {}
                    _ => return false,
                }
                return true;
            }
            InputEvent::Wheel {
                delta,
                ctrl_key: false,
            } => {
                let line = ambre_canvas::WHEEL_LINE_PX;
                let page = f64::from(self.list_height());
                let (_, dy) = delta.to_px((line, line), (page, page));
                let now_ms = (self.reads.now_ms)();
                let step = self.kinetic.wheel(0.0, dy as f32, delta.is_step(), now_ms);
                self.scroll_by(step);
                return true;
            }
            _ => {}
        }
        false
    }

    /// The share of `ev` the composer takes, and whether its default is
    /// prevented. `None` leaves it to the list.
    fn composer_input(&mut self, ev: &InputEvent) -> Option<bool> {
        let channel = self.writes_in()?;
        match *ev {
            InputEvent::Text { ref text } => self.composer.text(channel, text),
            InputEvent::CompositionUpdate { ref text } => {
                self.composer.compose(channel, text, Composition::Preview);
            }
            InputEvent::CompositionEnd { ref text } => {
                self.composer.compose(channel, text, Composition::Final)
            }
            // A press on the box under a thread's field checks it, and
            // leaves the keys where they were (`MURMURE-531`).
            InputEvent::PointerDown { y, button, .. }
                if button == PointerButton::Primary && self.composer.on_also(y as f32) =>
            {
                self.composer.toggle_also(channel);
            }
            InputEvent::DoubleClick { y, .. } if self.composer.on_also(y as f32) => {}
            // A press on a file of the draft takes it off (`MURMURE-132`).
            InputEvent::PointerDown { x, y, button, .. }
                if button == PointerButton::Primary
                    && self.composer.file_at(x as f32, y as f32).is_some() =>
            {
                if let Some(index) = self.composer.file_at(x as f32, y as f32) {
                    self.composer.uploads_mut().remove(channel, index);
                }
            }
            InputEvent::PointerDown { x, y, button, .. } if self.in_composer(y) => {
                self.hover = Some(Point::new(x, y));
                if button != PointerButton::Primary {
                    // No message is under the band.
                    (self.reads.act_on)(None);
                    return Some(false);
                }
                self.composer.press(channel, x as f32, y as f32);
                // The pipe takes the keys. The press is consumed, so the
                // canvas does not take the focus back.
                self.reads.pipe.focus();
            }
            InputEvent::PointerMove { x, y }
                if self.composer.drag_to(channel, x as f32, y as f32) =>
            {
                self.hover = Some(Point::new(x, y));
            }
            InputEvent::PointerUp { .. } if self.composer.release() => {}
            InputEvent::DoubleClick { x, y } if self.in_composer(y) => {
                self.composer.word_at(channel, x as f32, y as f32);
            }
            InputEvent::Key {
                ref key,
                shift,
                ctrl,
                alt,
                meta,
            } if self.composer.focused() => {
                // The control and the command keys are one accelerator, so
                // the composer's chords hold on every platform
                // (`UI-THREAD-028`).
                let mods = Modifiers::from_event(ctrl, alt, shift, meta);
                // Up in an empty composer edits the member's latest message,
                // or reply in a thread (`MURMURE-178`, `MURMURE-527`). With
                // none, it moves the caret as ever.
                if key == "ArrowUp"
                    && mods == Modifiers::NONE
                    && self.composer.blank(channel)
                    && let Some(edit) = self.last_own_edit(channel)
                {
                    self.composer.start_edit(edit);
                    return Some(true);
                }
                if mods == Modifiers::PRIMARY && key.chars().eq([EMOJI_KEY]) {
                    (self.reads.ask_emoji)(self.region);
                    return Some(true);
                }
                if !self.composer.key(channel, key, mods) {
                    return None;
                }
                // The box is drawn, so its turn by Alt+Enter is said
                // (`MURMURE-535`).
                if key == "Enter"
                    && mods.alt
                    && !mods.shift
                    && matches!(channel, DraftOf::Thread(..))
                {
                    let checked = self.composer.also_in_channel(channel);
                    (self.reads.announce)(&also_said(checked));
                }
            }
            _ => return None,
        }
        Some(true)
    }

    /// The text a copy or a cut puts on the clipboard. The composer's
    /// selection, while it holds the focus, else the list's (`MURMURE-426`).
    /// A cut takes the text out of the draft. The list's text stays.
    pub fn clipboard(&mut self, write: ClipboardWrite) -> Option<String> {
        if let Some(channel) = self.writes_in()
            && self.composer.focused()
        {
            if write == ClipboardWrite::Copy {
                return self.composer.selected(channel);
            }
            let text = self.composer.cut(channel);
            self.typed();
            self.dirty = true;
            return text;
        }
        self.copy().filter(|_| write == ClipboardWrite::Copy)
    }

    /// A paste lands in the composer of the region, as text.
    pub fn paste(&mut self, text: &str) {
        if let Some(channel) = self.writes_in()
            && !text.is_empty()
        {
            self.composer.text(channel, text);
            self.typed();
            self.dirty = true;
        }
    }

    /// A Murmure page binds no document, so a route swap changes nothing.
    pub fn set_document(&mut self, _document_id: DocumentId) {}
}

#[cfg(test)]
mod tests {
    use ambre_bus::{ByteSpan, CharSpan};
    use std::cell::Cell;

    use ambre_bus::conduit::local;
    use ambre_bus::{
        ChannelId, HeldReply, HeldSend, MediaRef, MediaType, Mention, MentionTarget, MessageView,
        MurmureEvent, StyleMark, TextStyle, Unreadable,
    };
    use ambre_canvas::theme::{FONT_BODY, FONT_UI, SPACE_2, SPACE_4, SPACE_8};
    use ambre_draw::kinetic::LONG_PRESS_MS;
    use ambre_draw::testing::RecordingSurface;

    use super::*;
    use crate::commands::REPLY;
    use crate::composer::ALSO_PX;
    use crate::list::tests::message;
    use ambre_bus::ThreadHead;

    struct Page {
        view: MurmureView,
        pane: Rc<Cell<MurmurePane>>,
        clock: Rc<Cell<ambre_core::PageTime>>,
        /// The wall clock, when a test sets it apart from the page's clock.
        wall: Rc<Cell<Option<Timestamp>>>,
        /// The user asks for reduced motion.
        still: Rc<Cell<bool>>,
        events: Sender<UiEvent>,
        commands: Receiver<LogicCommand>,
        /// What the view said to a screen reader, oldest first.
        said: Rc<RefCell<Vec<String>>>,
        /// Each target the view handed the page, oldest first.
        acted: Rc<RefCell<Vec<Option<MessageTarget>>>>,
        /// The edit the page holds for the view to take.
        editing: Rc<RefCell<Option<PendingEdit>>>,
        /// The signed-in member, `None` until a test names one.
        me: Rc<Cell<Option<UserId>>>,
        /// The members of every channel, as the page lists them.
        members: Rc<RefCell<Vec<(UserId, ambre_core::DisplayName)>>>,
        /// Each `@` list the view handed the page, oldest first.
        lists: Rc<RefCell<Vec<Listed>>>,
        /// The entry a click picked, for the region it names.
        picked: Rc<Cell<Option<(Region, OfferIndex)>>>,
        /// The send a cancelled `@channel` question gives back.
        returned: Rc<RefCell<Option<AskedAll>>>,
        /// Each address the view opened, oldest first.
        opened: Rc<RefCell<Vec<ambre_core::LinkUrl>>>,
        /// Each format bar the view handed the page, oldest first.
        bars: Rc<RefCell<Vec<Barred>>>,
        /// The style a press on the bar asked for, for the region it names.
        press: Rc<RefCell<Option<(Region, TextStyle)>>>,
        /// Each region the view opened the emoji picker for, oldest first.
        asked_emoji: Rc<RefCell<Vec<Region>>>,
        /// The emoji the picker picked, for the region it names.
        emoji: Rc<RefCell<Option<(Region, ambre_bus::Emoji)>>>,
        /// The files dropped on the canvas, for the view to take.
        dropped: Rc<RefCell<Vec<PickedFile>>>,
        /// Each file the view opened or saved, and its bytes, oldest first.
        saved: Rc<RefCell<Vec<Saved>>>,
        /// Each warning the view gave the user, oldest first.
        warned: Rc<RefCell<Vec<String>>>,
    }

    /// A file the view handed the page, and its bytes.
    type Saved = (Attachment, Vec<u8>);

    /// A format bar the view handed the page, and whether the thread's
    /// region shows it.
    type Barred = (Region, Option<MarkBar>);

    /// An `@` list the view handed the page, and whether the thread's region
    /// shows it.
    type Listed = (Region, Option<MentionList>);

    fn page() -> Page {
        let (events, from_logic) = local(64);
        let (to_logic, commands) = local(64);
        let pane = Rc::new(Cell::new(MurmurePane::default()));
        let shown = Rc::clone(&pane);
        let back = Rc::clone(&pane);
        let clock = Rc::new(Cell::new(ambre_core::PageTime(0.0)));
        let now = Rc::clone(&clock);
        let follows = Rc::clone(&clock);
        let wall_set = Rc::new(Cell::new(None::<Timestamp>));
        let wall = Rc::clone(&wall_set);
        let still = Rc::new(Cell::new(false));
        let reduced = Rc::clone(&still);
        let said_log = Rc::new(RefCell::new(Vec::new()));
        let said = Rc::clone(&said_log);
        let acted_log = Rc::new(RefCell::new(Vec::new()));
        let acted = Rc::clone(&acted_log);
        let editing = Rc::new(RefCell::new(None::<PendingEdit>));
        let asked = Rc::clone(&editing);
        let me = Rc::new(Cell::new(None));
        let signed_in = Rc::clone(&me);
        let members = Rc::new(RefCell::new(Vec::new()));
        let listed = Rc::clone(&members);
        let lists = Rc::new(RefCell::new(Vec::new()));
        let shown_lists = Rc::clone(&lists);
        let picked = Rc::new(Cell::new(None::<(Region, OfferIndex)>));
        let pick = Rc::clone(&picked);
        let returned = Rc::new(RefCell::new(None::<AskedAll>));
        let given = Rc::clone(&returned);
        let opened = Rc::new(RefCell::new(Vec::new()));
        let opens = Rc::clone(&opened);
        let bars = Rc::new(RefCell::new(Vec::new()));
        let shown_bars = Rc::clone(&bars);
        let press = Rc::new(RefCell::new(None::<(Region, TextStyle)>));
        let pressed = Rc::clone(&press);
        let asked_emoji = Rc::new(RefCell::new(Vec::new()));
        let asks = Rc::clone(&asked_emoji);
        let emoji = Rc::new(RefCell::new(None::<(Region, ambre_bus::Emoji)>));
        let picked_emoji = Rc::clone(&emoji);
        let dropped = Rc::new(RefCell::new(Vec::new()));
        let taken = Rc::clone(&dropped);
        let saved = Rc::new(RefCell::new(Vec::new()));
        let saves = Rc::clone(&saved);
        let warned = Rc::new(RefCell::new(Vec::new()));
        let warns = Rc::clone(&warned);
        let reads = PageReads {
            pane: Rc::new(move || shown.get()),
            name_of: Rc::new(|_| ambre_core::DisplayName::new("Ada").ok()),
            time_of: Rc::new(|_| "t".to_owned()),
            now_ms: Rc::new(move || now.get()),
            // The wall clock follows the page's unless a test sets it, so a
            // step dates what it sends.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            wall_clock: Rc::new(move || wall.get().unwrap_or(Timestamp(follows.get().0 as u64))),
            still: Rc::new(move || reduced.get()),
            announce: Rc::new(move |text: &str| said.borrow_mut().push(text.to_owned())),
            act_on: Rc::new(move |target| acted.borrow_mut().push(target)),
            take_edit: Rc::new(move |in_thread| {
                let mut asked = asked.borrow_mut();
                let reply = matches!(asked.as_ref()?.at, MessageAt::Thread(_));
                if reply == (in_thread == Region::Thread) {
                    asked.take()
                } else {
                    None
                }
            }),
            back: Rc::new(move || back.set(back.get().back())),
            me: Rc::new(move || signed_in.get()),
            pipe: TextInput::detached(),
            members_of: Rc::new(move |_| listed.borrow().clone()),
            show_mentions: Rc::new(move |in_thread, list| {
                shown_lists.borrow_mut().push((in_thread, list));
            }),
            take_pick: Rc::new(move |in_thread| match pick.get() {
                Some((region, index)) if region == in_thread => {
                    pick.set(None);
                    Some(index)
                }
                _ => None,
            }),
            show_bar: Rc::new(move |in_thread, bar| shown_bars.borrow_mut().push((in_thread, bar))),
            take_press: Rc::new(move |in_thread| {
                let mut pressed = pressed.borrow_mut();
                if pressed.as_ref()?.0 == in_thread {
                    pressed.take().map(|(_, style)| style)
                } else {
                    None
                }
            }),
            take_returned: Rc::new(move |in_thread| {
                let mut given = given.borrow_mut();
                if given.as_ref()?.region() == in_thread {
                    given.take()
                } else {
                    None
                }
            }),
            open_link: Rc::new(move |address: &ambre_core::LinkUrl| {
                opens.borrow_mut().push(address.clone())
            }),
            drafts: DraftShelf::default(),
            ask_emoji: Rc::new(move |in_thread| asks.borrow_mut().push(in_thread)),
            take_emoji: Rc::new(move |in_thread| {
                let mut picked = picked_emoji.borrow_mut();
                if picked.as_ref()?.0 == in_thread {
                    picked.take().map(|(_, emoji)| emoji)
                } else {
                    None
                }
            }),
            take_files: Rc::new(move || std::mem::take(&mut *taken.borrow_mut())),
            save_file: Rc::new(move |file: &Attachment, data| {
                saves.borrow_mut().push((file.clone(), data));
            }),
            warn: Rc::new(move |text: &str| warns.borrow_mut().push(text.to_owned())),
        };
        let size = (300.0, 400.0);
        let view = MurmureView::new(
            Some(from_logic),
            Some(to_logic),
            reads,
            size,
            ThemeControl::default(),
        );
        Page {
            wall: wall_set,
            view,
            pane,
            clock,
            still,
            events,
            commands,
            said: said_log,
            acted: acted_log,
            editing,
            me,
            members,
            lists,
            picked,
            returned,
            opened,
            bars,
            press,
            asked_emoji,
            emoji,
            dropped,
            saved,
            warned,
        }
    }

    /// Plan a frame and paint it when the plan asks, as the frame loop does.
    fn frame(view: &mut MurmureView) -> bool {
        frame_at(view, ambre_bus::PageTime(0.0))
    }

    fn frame_at(view: &mut MurmureView, now_ms: ambre_core::PageTime) -> bool {
        view.update();
        let plan = view.plan_repaint(now_ms, false);
        let full = matches!(plan.content, ContentRepaint::Full);
        if full {
            let (mut font_cx, mut cache) = (FontContext::new(), TextCache::new(1.0));
            let mut surface = RecordingSurface::new();
            view.paint_content(&mut surface, &mut font_cx, &mut cache, None);
        }
        full
    }

    fn send(page: &Page, channel_id: ChannelId, event: MurmureEvent) {
        let ev = UiEvent::Channel { channel_id, event };
        page.events.try_send(ev).expect("room in the queue");
    }

    fn list(page: &Page, channel: ChannelId, texts: &[&str]) -> SegmentId {
        let ada = UserId::new();
        let messages = texts
            .iter()
            .enumerate()
            .map(|(i, text)| message(ada, ambre_bus::Timestamp(i as u64), text))
            .collect();
        let segment = SegmentId::new();
        send(page, channel, MurmureEvent::Messages { segment, messages });
        segment
    }

    /// The mark reads sent since the last call: the channel's, then the
    /// thread's.
    fn reads(page: &Page) -> (usize, usize) {
        let mut reads = (0, 0);
        while let Ok(command) = page.commands.try_recv() {
            match command {
                LogicCommand::Channel {
                    command: MurmureCommand::MarkRead,
                    ..
                } => reads.0 += 1,
                LogicCommand::Channel {
                    command: MurmureCommand::MarkThreadRead { .. },
                    ..
                } => reads.1 += 1,
                _ => {}
            }
        }
        reads
    }

    fn laid_out(view: &MurmureView) -> Option<usize> {
        view.last_painted().map(|p| p.laid_out)
    }

    /// Selecting a channel shows its messages in the list, and its latest
    /// message shown marks it read once (`MURMURE-166`, `MURMURE-115`).
    #[test]
    fn the_selected_channel_shows_its_list_and_is_marked_read_once() {
        let mut page = page();
        let (shown, other) = (ChannelId::new(), ChannelId::new());
        list(&page, shown, &["a", "b"]);
        list(&page, other, &["elsewhere"]);
        assert!(frame(&mut page.view));
        assert_eq!(laid_out(&page.view), Some(0));
        assert_eq!(reads(&page).0, 0, "the channel list shows no channel");

        page.pane.set(MurmurePane::Channel {
            channel: shown,
            at: None,
        });
        assert!(frame(&mut page.view));
        assert_eq!(laid_out(&page.view), Some(2));
        assert_eq!(reads(&page).0, 1);
        assert!(!frame(&mut page.view), "nothing changed, nothing repaints");

        list(&page, shown, &["c"]);
        assert!(frame(&mut page.view));
        assert_eq!(laid_out(&page.view), Some(3));
        assert_eq!(reads(&page).0, 1, "a new latest message moves it again");
        page.view.resize(300, 400);
        assert!(frame(&mut page.view));
        assert_eq!(reads(&page).0, 0, "the same latest message moves it once");
    }

    /// The thread's region shows the thread from the events the content
    /// canvas drains (`MURMURE-420`). It marks the thread read once per last
    /// reply (`MURMURE-497`).
    #[test]
    fn the_thread_region_shows_the_thread_through_the_same_list() {
        let mut page = page();
        let channel = ChannelId::new();
        let ada = UserId::new();
        let root = message(ada, ambre_bus::Timestamp(0), "root");
        let segment = SegmentId::new();
        let messages = vec![root.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let replies =
            ["one", "two", "three"].map(|text| message(ada, ambre_bus::Timestamp(1), text));
        let thread = DocumentId::new();
        let ev = MurmureEvent::ThreadMessages {
            root: root.id,
            thread,
            messages: replies.to_vec(),
        };
        send(&page, channel, ev);
        let mut thread_view = page.view.region(300.0, 400.0);
        page.pane.set(MurmurePane::Thread {
            channel,
            root: MessageRef {
                segment,
                message: root.id,
            },
        });
        assert!(frame(&mut page.view));
        assert!(frame(&mut thread_view));
        assert_eq!(laid_out(&page.view), Some(1));
        assert_eq!(laid_out(&thread_view), Some(3));
        assert_eq!(reads(&page), (1, 1), "each region moves its own cursor");
        assert!(!frame(&mut thread_view));
        assert_eq!(reads(&page), (0, 0));

        let ev = MurmureEvent::ThreadMessages {
            root: root.id,
            thread,
            messages: vec![message(ada, ambre_bus::Timestamp(4), "four")],
        };
        send(&page, channel, ev);
        page.view.update();
        assert!(frame(&mut thread_view), "the thread repaints on its reply");
        assert_eq!(laid_out(&thread_view), Some(4));
        assert_eq!(reads(&page), (0, 1), "a new last reply marks it again");
    }

    /// The unread line shows above the first unread message, and stays
    /// after the list marks the channel read (`MURMURE-184`).
    #[test]
    fn the_unread_line_stays_after_the_channel_is_marked_read() {
        let mut page = page();
        let channel = ChannelId::new();
        let ada = UserId::new();
        let (read, unread) = (
            message(ada, ambre_bus::Timestamp(0), "a"),
            message(ada, ambre_bus::Timestamp(1), "b"),
        );
        let first = Some(unread.id);
        let segment = SegmentId::new();
        let messages = vec![read, unread];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        send(&page, channel, MurmureEvent::FirstUnread { message: first });
        page.pane.set(MurmurePane::Channel { channel, at: None });
        let line = |view: &MurmureView| view.last_painted().map(|p| p.line_shown);
        assert!(frame(&mut page.view));
        assert_eq!(line(&page.view), Some(true));
        assert_eq!(reads(&page).0, 1);

        // The Logic thread answers the mark read: nothing is unread now.
        send(&page, channel, MurmureEvent::FirstUnread { message: None });
        assert!(frame(&mut page.view));
        assert_eq!(line(&page.view), Some(true), "held while on screen");

        page.pane.set(MurmurePane::ChannelList);
        assert!(frame(&mut page.view));
        page.pane.set(MurmurePane::Channel { channel, at: None });
        assert!(frame(&mut page.view));
        assert_eq!(line(&page.view), Some(false), "back, all is read");
    }

    /// Plan a frame and paint it with the faces the page ships, so a point
    /// falls on the bytes a reader sees there.
    fn frame_with_fonts(view: &mut MurmureView) {
        view.update();
        view.plan_repaint(ambre_bus::PageTime(0.0), false);
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let mut surface = RecordingSurface::new();
        view.paint_content(&mut surface, &mut font_cx, &mut cache, None);
    }

    fn mouse(view: &mut MurmureView, down: (f64, f64), up: (f64, f64)) {
        let (x, y) = down;
        let press = InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        };
        assert!(!view.handle_input(&press), "the press is the page's");
        assert!(view.drags_text());
        let (x, y) = up;
        if up != down {
            view.handle_input(&InputEvent::PointerMove { x, y });
        }
        let release = InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Primary,
            ctrl: false,
        };
        view.handle_input(&release);
        assert!(!view.drags_text());
    }

    /// `MURMURE-425` to `MURMURE-427`: a mouse drag across two messages
    /// selects their text, and a copy reads each under its author and its
    /// time. Neither the drag, its paints nor the copy send a command, and
    /// a click clears the selection.
    #[test]
    fn a_drag_selects_text_that_a_copy_reads_and_sends_nothing() {
        let mut page = page();
        let channel = ChannelId::new();
        list(&page, channel, &["hello", "world"]);
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        assert_eq!(reads(&page).0, 1);
        let rows = page.view.last_painted().expect("painted").rows;
        let [(_, _, first), (_, top, second)] = rows.as_slice() else {
            panic!("two rows: {rows:?}");
        };
        let press = (f64::from(SPACE_8), f64::from(first - SPACE_2 - 2.0));
        let release = (290.0, f64::from((top + second) / 2.0));
        mouse(&mut page.view, press, release);
        frame_with_fonts(&mut page.view);
        let copied = page.view.copy();
        assert_eq!(copied.as_deref(), Some("Ada, t\nhello\n\nAda, t\nworld"));
        assert!(
            page.commands.try_recv().is_err(),
            "the selection sent nothing"
        );

        mouse(&mut page.view, press, press);
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.copy(), None, "a click clears it");
        assert!(page.commands.try_recv().is_err());
    }

    /// A click or a tap on a link opens its address, and sends nothing
    /// (`MURMURE-567`, `MURMURE-568`). A click beside it, a drag over it and
    /// an address the write rule refuses open nothing.
    #[test]
    fn a_click_on_a_link_opens_its_address_and_sends_nothing() {
        let mut page = page();
        let channel = ChannelId::new();
        let link = |text: &str, address: &str, range| {
            let mut sent = message(UserId::new(), ambre_bus::Timestamp(0), text);
            sent.styles = vec![StyleMark {
                style: TextStyle::Link(address.into()),
                range,
            }];
            sent
        };
        let messages = vec![
            link(
                "bad x",
                "javascript:alert(1)",
                CharSpan::from_usize_range(0..3),
            ),
            link(
                "see site now",
                "https://ambre.dev",
                CharSpan::from_usize_range(4..8),
            ),
        ];
        let segment = SegmentId::new();
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        assert_eq!(reads(&page).0, 1);
        let rows = page.view.last_painted().expect("painted").rows;
        let [(_, _, first), (_, _, second)] = rows.as_slice() else {
            panic!("two rows: {rows:?}");
        };
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let mut x_after = |text| {
            let width =
                crate::paint::shape(&mut font_cx, &mut cache, text, FONT_BODY, 284.0, false);
            f64::from(SPACE_8 + width.width())
        };
        let on_link = (x_after("see si"), f64::from(second - SPACE_2 - 6.0));
        let beside = (x_after("s"), on_link.1);
        let on_bad = (x_after("b"), f64::from(first - SPACE_2 - 6.0));

        for at in [beside, on_bad] {
            mouse(&mut page.view, at, at);
            frame_with_fonts(&mut page.view);
        }
        mouse(&mut page.view, beside, on_link);
        frame_with_fonts(&mut page.view);
        assert!(page.view.copy().is_some(), "the drag selects");
        assert!(page.opened.borrow().is_empty(), "nothing opened");

        mouse(&mut page.view, on_link, on_link);
        frame_with_fonts(&mut page.view);
        page.view
            .touch_down(on_link.0 as f32, on_link.1 as f32, ambre_bus::PageTime(0.0));
        assert!(page.view.touch_up(ambre_bus::PageTime(50.0)).is_some());
        frame_with_fonts(&mut page.view);
        let opened = page.opened.borrow().clone();
        assert_eq!(
            opened,
            [
                ambre_core::LinkUrl::from("https://ambre.dev"),
                ambre_core::LinkUrl::from("https://ambre.dev")
            ]
        );
        assert!(page.commands.try_recv().is_err(), "the opens sent nothing");
    }

    /// A click on a reaction chip adds the member's reaction when theirs is
    /// not in it, and takes it away when it is (`MURMURE-140`).
    #[test]
    fn a_click_on_a_chip_adds_or_takes_away_the_members_reaction() {
        let mut page = page();
        let (me, ada) = (UserId::new(), UserId::new());
        page.me.set(Some(me));
        let channel = ChannelId::new();
        let mut reacted = message(ada, ambre_bus::Timestamp(1), "hello");
        reacted.reactions = vec![
            ambre_bus::Reaction {
                emoji: ambre_bus::Emoji::from("\u{1F44D}"),
                users: vec![ada],
            },
            ambre_bus::Reaction {
                emoji: ambre_bus::Emoji::from("\u{1F389}"),
                users: vec![me],
            },
        ];
        let id = reacted.id;
        let segment = SegmentId::new();
        let messages = vec![reacted];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        reads(&page);
        let rows = page.view.last_painted().expect("painted").rows;
        let [(_, _, bottom)] = rows.as_slice() else {
            panic!("one row: {rows:?}");
        };
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let first = crate::paint::shape(
            &mut font_cx,
            &mut cache,
            "\u{1F44D} 1",
            FONT_UI,
            284.0,
            false,
        );
        let y = f64::from(bottom - 2.0 * SPACE_2 - 2.0);
        let on_first = (f64::from(SPACE_8 + 2.0), y);
        let on_second = (f64::from(SPACE_8 + first.width() + 4.0 * SPACE_4), y);
        let mut clicked = Vec::new();
        for at in [on_first, on_second] {
            mouse(&mut page.view, at, at);
            frame_with_fonts(&mut page.view);
            while let Ok(command) = page.commands.try_recv() {
                if let LogicCommand::Channel { command, .. } = command {
                    clicked.push(command);
                }
            }
        }
        let at = MessageAt::Segment(segment);
        let [
            MurmureCommand::React {
                at: added_at,
                message: added,
                emoji: thumbs,
                ..
            },
            MurmureCommand::Unreact {
                at: removed_at,
                message: removed,
                emoji: party,
            },
        ] = clicked.as_slice()
        else {
            panic!("one reaction added, one taken away: {clicked:?}");
        };
        assert_eq!((*added_at, *added, thumbs.as_str()), (at, id, "\u{1F44D}"));
        assert_eq!(
            (*removed_at, *removed, party.as_str()),
            (at, id, "\u{1F389}")
        );
    }

    /// Ctrl and `;` in the composer opens the picker for its region, and a
    /// pick goes in at the caret of that region alone (`MURMURE-139`).
    #[test]
    fn a_picked_emoji_goes_in_at_the_caret() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "ab");
        key(&mut page, "ArrowLeft", false, ambre_bus::PageTime(0.0));
        assert!(key(&mut page, ";", true, ambre_bus::PageTime(0.0)));
        assert_eq!(*page.asked_emoji.borrow(), [Region::Channel]);
        *page.emoji.borrow_mut() = Some((Region::Thread, ambre_bus::Emoji::from("\u{1F680}")));
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().draft(channel).as_deref(), Some("ab"));
        *page.emoji.borrow_mut() = Some((Region::Channel, ambre_bus::Emoji::from("\u{1F680}")));
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.composer().draft(channel).as_deref(),
            Some("a\u{1F680}b")
        );
        assert!(page.emoji.borrow().is_none(), "the pick is taken once");
    }

    /// A drag past the top of the list scrolls it under the pointer, and a
    /// drag inside it does not.
    #[test]
    fn a_drag_past_the_top_scrolls_the_list_under_it() {
        let mut page = page();
        let channel = ChannelId::new();
        list(&page, channel, &["hello"; 100]);
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame(&mut page.view);
        mouse(&mut page.view, (10.0, 200.0), (10.0, 300.0));
        frame(&mut page.view);
        assert_eq!(page.view.resting_on(), None, "still on the latest row");
        mouse(&mut page.view, (10.0, 200.0), (10.0, -50.0));
        frame(&mut page.view);
        assert!(page.view.resting_on().is_some(), "the list moved up");
    }

    /// A wheel event at `now_ms`, `delta_mode` as the browser names it.
    fn wheel(page: &mut Page, dy: f64, delta_mode: u32, now_ms: ambre_core::PageTime) -> bool {
        page.clock.set(now_ms);
        let ev = InputEvent::Wheel {
            delta: ambre_canvas::WheelDelta::new(
                0.0,
                dy,
                ambre_canvas::WheelDeltaMode::from_dom(delta_mode),
            ),
            ctrl_key: false,
        };
        page.view.handle_input(&ev)
    }

    /// `count` messages of one author, written from `from` on, in one new
    /// segment of `channel`.
    fn segment(page: &Page, channel: ChannelId, from: u64, count: u64) -> Vec<MessageView> {
        let ada = UserId::new();
        let messages: Vec<_> = (from..from + count)
            .map(|ts| message(ada, ambre_bus::Timestamp(ts), "hello"))
            .collect();
        let ev = MurmureEvent::Messages {
            segment: SegmentId::new(),
            messages: messages.clone(),
        };
        send(page, channel, ev);
        messages
    }

    /// The segments the view asked to load before, since the last call.
    fn older(page: &Page) -> Vec<SegmentId> {
        let mut asked = Vec::new();
        while let Ok(command) = page.commands.try_recv() {
            if let LogicCommand::Channel {
                command: MurmureCommand::LoadOlder { segment },
                ..
            } = command
            {
                asked.push(segment);
            }
        }
        asked
    }

    fn shown(view: &MurmureView) -> Painted {
        view.last_painted().expect("a paint")
    }

    /// A channel of `count` messages on screen, at its latest.
    fn long_channel(count: u64) -> (Page, ChannelId, Vec<MessageView>) {
        let page = page();
        let channel = ChannelId::new();
        let messages = segment(&page, channel, 1_000, count);
        page.pane.set(MurmurePane::Channel { channel, at: None });
        (page, channel, messages)
    }

    /// At the bottom, a new message scrolls into view and is read
    /// (`MURMURE-189`).
    #[test]
    fn a_new_message_scrolls_into_view_at_the_bottom() {
        let (mut page, channel, _) = long_channel(200);
        assert!(frame(&mut page.view));
        assert_eq!(page.view.resting_on(), None);
        assert_eq!(reads(&page).0, 1);
        segment(&page, channel, 5_000, 1);
        assert!(frame(&mut page.view));
        assert!(shown(&page.view).latest_shown);
        assert_eq!(page.view.resting_on(), None);
        assert_eq!(reads(&page).0, 1, "the new message shows, and is read");
    }

    /// Higher up, a new message moves nothing on screen and marks nothing
    /// read (`MURMURE-190`).
    #[test]
    fn a_new_message_leaves_a_reader_higher_up_in_place() {
        let (mut page, channel, _) = long_channel(200);
        frame(&mut page.view);
        reads(&page);
        assert!(
            wheel(&mut page, -3.0, 1, ambre_bus::PageTime(10.0)),
            "the wheel is taken"
        );
        assert!(frame(&mut page.view));
        let rested = page.view.resting_on().expect("resting higher up");
        assert!(!shown(&page.view).latest_shown);
        segment(&page, channel, 5_000, 1);
        assert!(frame(&mut page.view));
        assert_eq!(page.view.resting_on(), Some(rested));
        assert_eq!(reads(&page).0, 0, "the latest message is not on screen");
    }

    /// Reaching the oldest row asks for the segment before it, once per
    /// segment (`MURMURE-191`). The rows it brings land above, and nothing
    /// on screen moves (`MURMURE-192`).
    #[test]
    fn the_oldest_row_asks_for_the_segment_before_and_its_rows_move_nothing() {
        let (mut page, channel, _) = long_channel(200);
        frame(&mut page.view);
        assert_eq!(older(&page), [], "the oldest row is far above");
        wheel(&mut page, -100_000.0, 0, ambre_bus::PageTime(10.0));
        assert!(frame(&mut page.view));
        assert!(shown(&page.view).oldest_shown);
        let oldest = page
            .view
            .lists
            .borrow()
            .conversations
            .channel(channel)
            .and_then(|l| l.oldest_segment());
        assert_eq!(older(&page), oldest.into_iter().collect::<Vec<_>>());
        wheel(&mut page, -50.0, 0, ambre_bus::PageTime(20.0));
        assert!(frame(&mut page.view));
        assert_eq!(older(&page), [], "asked once for this segment");
        let rested = page.view.resting_on().expect("resting at the top");

        segment(&page, channel, 0, 50);
        assert!(frame(&mut page.view));
        assert_eq!(
            page.view.resting_on(),
            Some(rested),
            "the rows above move nothing"
        );
        assert!(!shown(&page.view).oldest_shown);
        wheel(&mut page, -100_000.0, 0, ambre_bus::PageTime(30.0));
        assert!(frame(&mut page.view));
        assert_eq!(older(&page).len(), 1, "the new oldest segment asks again");
    }

    /// A trackpad glide that pauses keeps the list moving through the
    /// shared model; a notch never does (`MURMURE-417`).
    #[test]
    fn a_paused_glide_coasts_and_a_notch_never_does() {
        let (mut page, _, _) = long_channel(400);
        frame(&mut page.view);
        wheel(&mut page, -2_000.0, 0, ambre_bus::PageTime(10.0));
        frame_at(&mut page.view, ambre_bus::PageTime(10.0));
        for (i, now) in [1_000.0, 1_010.0, 1_020.0].into_iter().enumerate() {
            wheel(&mut page, -10.0, 0, ambre_bus::PageTime(now));
            frame_at(&mut page.view, ambre_bus::PageTime(now + i as f64));
        }
        let paused = page.view.resting_on();
        frame_at(&mut page.view, ambre_bus::PageTime(1_100.0));
        assert!(page.view.scrolling());
        frame_at(&mut page.view, ambre_bus::PageTime(1_116.0));
        frame_at(&mut page.view, ambre_bus::PageTime(1_132.0));
        assert_ne!(page.view.resting_on(), paused, "the glide coasts on");

        page.view.kinetic.stop();
        wheel(&mut page, -1.0, 1, ambre_bus::PageTime(2_000.0));
        frame_at(&mut page.view, ambre_bus::PageTime(2_000.0));
        let notched = page.view.resting_on();
        assert!(!page.view.scrolling());
        frame_at(&mut page.view, ambre_bus::PageTime(2_200.0));
        assert_eq!(page.view.resting_on(), notched, "a notch leaves no motion");
    }

    /// A motion that meets the latest row stops there.
    #[test]
    fn a_motion_stops_at_the_latest_row() {
        let (mut page, _, _) = long_channel(400);
        frame(&mut page.view);
        wheel(&mut page, -60.0, 0, ambre_bus::PageTime(10.0));
        frame_at(&mut page.view, ambre_bus::PageTime(10.0));
        for now in [1_000.0, 1_010.0, 1_020.0] {
            wheel(&mut page, 10.0, 0, ambre_bus::PageTime(now));
        }
        let mut now = 1_100.0;
        while now < 1_300.0 {
            frame_at(&mut page.view, ambre_bus::PageTime(now));
            now += 16.0;
        }
        assert_eq!(page.view.resting_on(), None);
        assert!(!page.view.scrolling(), "the edge stops the motion");
    }

    /// A key pressed at `now_ms`, with `ctrl` held or not.
    fn key(page: &mut Page, key: &str, ctrl: bool, now_ms: ambre_core::PageTime) -> bool {
        page.clock.set(now_ms);
        let ev = InputEvent::Key {
            key: key.to_owned(),
            ctrl,
            alt: false,
            shift: false,
            meta: false,
        };
        page.view.handle_input(&ev)
    }

    /// Page Up rests the list where a wheel step of one page does, through
    /// the same model, and Page Down brings it back (`MURMURE-504`). A key
    /// with a modifier stays the browser's.
    #[test]
    fn page_up_and_page_down_move_the_list_by_one_viewport() {
        let (mut paged, _, paged_rows) = long_channel(400);
        let (mut wheeled, _, wheeled_rows) = long_channel(400);
        frame(&mut paged.view);
        frame(&mut wheeled.view);
        assert!(key(&mut paged, "PageUp", false, ambre_bus::PageTime(10.0)));
        assert!(wheel(&mut wheeled, -1.0, 2, ambre_bus::PageTime(10.0)));
        frame_at(&mut paged.view, ambre_bus::PageTime(10.0));
        frame_at(&mut wheeled.view, ambre_bus::PageTime(10.0));
        // The two lists hold the same rows under other ids: compare places.
        let place = |view: &MurmureView, rows: &[MessageView]| {
            let (key, y) = view.resting_on().expect("a page up leaves the latest row");
            let index = rows.iter().position(|m| RowKey::Message(m.id) == key);
            (index.expect("a message row"), y)
        };
        assert_eq!(
            place(&paged.view, &paged_rows),
            place(&wheeled.view, &wheeled_rows)
        );
        assert!(!paged.view.scrolling(), "a page leaves no motion");

        assert!(key(
            &mut paged,
            "PageDown",
            false,
            ambre_bus::PageTime(20.0)
        ));
        frame_at(&mut paged.view, ambre_bus::PageTime(20.0));
        assert!(shown(&paged.view).latest_shown);
        assert!(
            !key(&mut paged, "PageUp", true, ambre_bus::PageTime(30.0)),
            "the browser's chord"
        );
        assert!(!key(&mut paged, "a", false, ambre_bus::PageTime(30.0)));
    }

    /// End, a click on the control or a tap on it brings back the latest
    /// message. The control shows only while the list rests higher up
    /// (`MURMURE-505`).
    #[test]
    fn end_or_the_control_brings_back_the_latest_message() {
        let (mut page, _, _) = long_channel(400);
        frame(&mut page.view);
        assert_eq!(page.view.jump, None, "no control at the latest message");
        let centre = |view: &MurmureView| view.jump.expect("a control").center();
        let up = |page: &mut Page| {
            key(page, "PageUp", false, ambre_bus::PageTime(10.0));
            frame(&mut page.view);
            assert!(!shown(&page.view).latest_shown);
        };

        up(&mut page);
        assert!(key(&mut page, "End", false, ambre_bus::PageTime(20.0)));
        frame(&mut page.view);
        assert!(shown(&page.view).latest_shown);
        assert_eq!(page.view.resting_on(), None);
        assert_eq!(page.view.jump, None);

        up(&mut page);
        let at = centre(&page.view);
        let click = InputEvent::PointerUp {
            x: at.x,
            y: at.y,
            button: PointerButton::Primary,
            ctrl: false,
        };
        assert!(page.view.handle_input(&click), "the click is taken");
        frame(&mut page.view);
        assert!(shown(&page.view).latest_shown);

        up(&mut page);
        let at = centre(&page.view);
        page.view
            .touch_down(at.x as f32, at.y as f32, ambre_bus::PageTime(100.0));
        assert_eq!(
            page.view.touch_up(ambre_bus::PageTime(150.0)),
            None,
            "the tap is taken"
        );
        frame(&mut page.view);
        assert!(shown(&page.view).latest_shown);
    }

    /// A jump marks read only the latest message it showed. Pages that stop
    /// short of a new message mark nothing, and End marks it once it paints
    /// (`MURMURE-506`).
    #[test]
    fn a_jump_marks_read_only_what_it_showed() {
        let (mut page, channel, _) = long_channel(400);
        frame(&mut page.view);
        assert_eq!(reads(&page).0, 1);
        for now in [10.0, 20.0, 30.0] {
            key(&mut page, "PageUp", false, ambre_bus::PageTime(now));
            frame_at(&mut page.view, ambre_bus::PageTime(now));
        }
        segment(&page, channel, 5_000, 1);
        key(&mut page, "PageDown", false, ambre_bus::PageTime(40.0));
        frame_at(&mut page.view, ambre_bus::PageTime(40.0));
        assert!(!shown(&page.view).latest_shown);
        assert_eq!(
            reads(&page).0,
            0,
            "the page stopped short of the new message"
        );
        key(&mut page, "End", false, ambre_bus::PageTime(50.0));
        assert_eq!(reads(&page).0, 0, "nothing is read before it paints");
        frame_at(&mut page.view, ambre_bus::PageTime(50.0));
        assert_eq!(reads(&page).0, 1, "the new message showed, and is read");
    }

    /// The key of the message at `index` of `rows`.
    fn row(rows: &[MessageView], index: usize) -> Option<RowKey> {
        rows.get(index).map(|m| RowKey::Message(m.id))
    }

    /// Up and Down move the focus one message at a time, from the last row
    /// on screen. A screen reader hears each one it reaches. The list
    /// follows the focus up, and keeps the focused row on screen
    /// (`MURMURE-205`, `MURMURE-207`).
    #[test]
    fn up_and_down_move_the_focus_and_say_each_message() {
        let (mut page, _, rows) = long_channel(400);
        frame(&mut page.view);
        assert_eq!(page.view.focused(), None);
        assert!(
            key(&mut page, "ArrowUp", false, ambre_bus::PageTime(10.0)),
            "the key is taken"
        );
        assert_eq!(
            page.view.focused(),
            row(&rows, 399),
            "the last row on screen"
        );
        assert_eq!(
            page.said.borrow().last().map(String::as_str),
            Some("Ada, t: hello")
        );
        key(&mut page, "ArrowUp", false, ambre_bus::PageTime(20.0));
        assert_eq!(page.view.focused(), row(&rows, 398));
        key(&mut page, "ArrowDown", false, ambre_bus::PageTime(30.0));
        key(&mut page, "ArrowDown", false, ambre_bus::PageTime(40.0));
        assert_eq!(
            page.view.focused(),
            row(&rows, 399),
            "the latest is the end"
        );
        assert_eq!(page.said.borrow().len(), 4, "each move is said");
        for step in 0..60 {
            key(
                &mut page,
                "ArrowUp",
                false,
                ambre_bus::PageTime(50.0 + f64::from(step)),
            );
            frame(&mut page.view);
            frame(&mut page.view);
            let (top, bottom) = shown(&page.view).focus.expect("the focus on screen");
            assert!(top >= -1.0 && bottom <= 401.0, "{step}: {top} {bottom}");
        }
        assert_eq!(page.view.focused(), row(&rows, 339));
        assert!(
            !shown(&page.view).latest_shown,
            "the list followed the focus up"
        );
    }

    /// Moving the focus moves no read cursor: a list higher up, short of a
    /// new message, reads nothing however the focus moves (`MURMURE-206`).
    #[test]
    fn moving_the_focus_moves_no_read_cursor() {
        let (mut page, channel, _) = long_channel(400);
        frame(&mut page.view);
        assert_eq!(reads(&page).0, 1);
        for now in [10.0, 20.0] {
            key(&mut page, "PageUp", false, ambre_bus::PageTime(now));
            frame_at(&mut page.view, ambre_bus::PageTime(now));
        }
        segment(&page, channel, 5_000, 1);
        for (step, arrow) in ["ArrowUp", "ArrowUp", "ArrowDown", "ArrowDown", "ArrowDown"]
            .into_iter()
            .enumerate()
        {
            key(
                &mut page,
                arrow,
                false,
                ambre_bus::PageTime(30.0 + step as f64),
            );
            frame(&mut page.view);
        }
        assert!(page.view.focused().is_some());
        assert!(!shown(&page.view).latest_shown);
        assert_eq!(reads(&page).0, 0, "the focus read nothing");
    }

    /// At the bottom a new message is said, and the focus stays where it
    /// was. The list a channel first shows is not news, and a reader higher
    /// up hears no message (`MURMURE-208`, `MURMURE-209`).
    #[test]
    fn a_new_message_is_said_at_the_bottom_and_leaves_the_focus() {
        let (mut page, channel, _) = long_channel(50);
        frame(&mut page.view);
        assert!(page.said.borrow().is_empty(), "the first list is not news");
        key(&mut page, "ArrowUp", false, ambre_bus::PageTime(10.0));
        frame(&mut page.view);
        let focus = page.view.focused();
        page.said.borrow_mut().clear();
        segment(&page, channel, 5_000, 1);
        frame(&mut page.view);
        assert_eq!(*page.said.borrow(), ["Ada, t: hello"]);
        assert_eq!(page.view.focused(), focus, "the focus did not move");
        assert_eq!(page.view.resting_on(), None);

        key(&mut page, "PageUp", false, ambre_bus::PageTime(20.0));
        frame_at(&mut page.view, ambre_bus::PageTime(20.0));
        page.said.borrow_mut().clear();
        segment(&page, channel, 6_000, 1);
        frame(&mut page.view);
        assert!(
            page.said.borrow().is_empty(),
            "a reader higher up hears none"
        );
    }

    /// The control back to the latest message is said once each time it
    /// shows, with the key that reaches it (`MURMURE-509`).
    #[test]
    fn the_control_is_said_with_its_key_each_time_it_shows() {
        let (mut page, _, _) = long_channel(400);
        frame(&mut page.view);
        assert!(page.said.borrow().is_empty());
        key(&mut page, "PageUp", false, ambre_bus::PageTime(10.0));
        frame_at(&mut page.view, ambre_bus::PageTime(10.0));
        key(&mut page, "PageUp", false, ambre_bus::PageTime(20.0));
        frame_at(&mut page.view, ambre_bus::PageTime(20.0));
        assert_eq!(*page.said.borrow(), [JUMP_HINT]);
        assert!(JUMP_HINT.contains("End"));
        assert!(key(&mut page, "End", false, ambre_bus::PageTime(30.0)));
        frame_at(&mut page.view, ambre_bus::PageTime(30.0));
        key(&mut page, "PageUp", false, ambre_bus::PageTime(40.0));
        frame_at(&mut page.view, ambre_bus::PageTime(40.0));
        assert_eq!(*page.said.borrow(), [JUMP_HINT, JUMP_HINT]);
    }

    /// The thread's list takes the keys the channel's list takes, and says
    /// what it reaches. Its keys leave the channel's focus alone
    /// (`MURMURE-510`).
    #[test]
    fn the_thread_takes_the_keys_of_the_channel() {
        let mut page = page();
        let channel = ChannelId::new();
        let ada = UserId::new();
        let root = message(ada, ambre_bus::Timestamp(0), "root");
        let segment = SegmentId::new();
        let messages = vec![root.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let thread = DocumentId::new();
        let replies: Vec<_> = (1..=200)
            .map(|ts| message(ada, ambre_bus::Timestamp(ts), "hello"))
            .collect();
        let ev = MurmureEvent::ThreadMessages {
            root: root.id,
            thread,
            messages: replies.clone(),
        };
        send(&page, channel, ev);
        let at = MessageRef {
            segment,
            message: root.id,
        };
        page.pane.set(MurmurePane::Thread { channel, root: at });
        let mut view = page.view.region(300.0, 200.0);
        frame(&mut page.view);
        frame(&mut view);
        assert!(page.said.borrow().is_empty(), "the first list is not news");

        let clock = Rc::clone(&page.clock);
        let press = |view: &mut MurmureView, key: &str, now_ms: ambre_core::PageTime| {
            clock.set(now_ms);
            let taken = view.handle_input(&InputEvent::Key {
                key: key.to_owned(),
                ctrl: false,
                alt: false,
                shift: false,
                meta: false,
            });
            frame_at(view, now_ms);
            taken
        };
        assert!(press(&mut view, "ArrowUp", ambre_bus::PageTime(10.0)));
        assert_eq!(view.focused(), row(&replies, 199), "the last reply");
        assert_eq!(*page.said.borrow(), ["Ada, t: hello"]);
        assert!(press(&mut view, "PageUp", ambre_bus::PageTime(20.0)));
        press(&mut view, "PageUp", ambre_bus::PageTime(30.0));
        assert!(!shown(&view).latest_shown, "the pages moved the thread");
        assert_eq!(
            page.said.borrow().last().map(String::as_str),
            Some(JUMP_HINT)
        );
        assert!(press(&mut view, "End", ambre_bus::PageTime(40.0)));
        assert!(shown(&view).latest_shown, "End is back at the latest reply");
        assert_eq!(page.view.focused(), None, "the channel kept its own focus");

        page.said.borrow_mut().clear();
        let ev = MurmureEvent::ThreadMessages {
            root: root.id,
            thread,
            messages: vec![message(ada, ambre_bus::Timestamp(500), "hello")],
        };
        send(&page, channel, ev);
        frame(&mut page.view);
        frame(&mut view);
        assert_eq!(
            *page.said.borrow(),
            ["Ada, t: hello"],
            "a new reply is said"
        );
    }

    /// A finger drags the list, and one that never left the slop is a tap
    /// (`MURMURE-417`).
    #[test]
    fn a_finger_drags_the_list_and_a_still_one_taps() {
        let (mut page, _, _) = long_channel(200);
        frame(&mut page.view);
        page.view.touch_down(10.0, 100.0, ambre_bus::PageTime(0.0));
        assert!(
            !page.view.touch_move(10.0, 102.0, ambre_bus::PageTime(10.0)),
            "inside the slop"
        );
        assert!(page.view.touch_move(10.0, 300.0, ambre_bus::PageTime(20.0)));
        frame_at(&mut page.view, ambre_bus::PageTime(20.0));
        assert!(
            page.view.resting_on().is_some(),
            "the list followed the finger"
        );
        page.view.touch_cancel();
        assert!(!page.view.scrolling());
        page.view
            .touch_down(10.0, 100.0, ambre_bus::PageTime(100.0));
        assert_eq!(
            page.view.touch_up(ambre_bus::PageTime(150.0)),
            Some((10.0, 100.0))
        );
        page.view
            .touch_down(10.0, 100.0, ambre_bus::PageTime(200.0));
        page.view
            .touch_move(10.0, 300.0, ambre_bus::PageTime(300.0));
        assert_eq!(page.view.touch_up(ambre_bus::PageTime(1_000.0)), None);
    }

    /// The middle height of the row at `index`, as the last paint placed it.
    fn middle_of(view: &MurmureView, index: usize) -> f32 {
        let painted = shown(view);
        let &(_, top, bottom) = painted
            .rows
            .iter()
            .find(|&&(i, _, _)| i == index)
            .expect("the row on screen");
        (top + bottom) / 2.0
    }

    fn button(page: &mut Page, button: PointerButton, y: f32) -> bool {
        let ev = InputEvent::PointerDown {
            x: 10.0,
            y: y.into(),
            button,
            shift: false,
            ctrl: false,
        };
        page.view.handle_input(&ev)
    }

    /// A channel shown with a band for `cause` at its top, then two messages.
    /// Returns the page, the channel and the segment of the band.
    fn banded(cause: Unreadable) -> (Page, ChannelId, SegmentId) {
        let mut page = page();
        let channel = ChannelId::new();
        let ada = UserId::new();
        let messages = vec![
            message(ada, ambre_bus::Timestamp(10), "a"),
            message(ada, ambre_bus::Timestamp(11), "b"),
        ];
        let segment = SegmentId::new();
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let older = SegmentId::new();
        let band = MurmureEvent::Unreadable {
            segment: older,
            cause,
            from: Timestamp(0),
            to: Some(Timestamp(10)),
        };
        send(&page, channel, band);
        page.pane.set(MurmurePane::channel(channel));
        frame(&mut page.view);
        let _ = reads(&page);
        (page, channel, older)
    }

    /// The segments the view asked for again since the last call.
    fn retried(page: &Page) -> Vec<(ChannelId, SegmentId)> {
        let mut asked = Vec::new();
        while let Ok(command) = page.commands.try_recv() {
            if let LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::Retry { segment },
            } = command
            {
                asked.push((channel_id, segment));
            }
        }
        asked
    }

    /// A click, a tap or Enter on a band asks for its segment again, and a
    /// click on a message asks for nothing (`MURMURE-513`).
    #[test]
    fn a_click_a_tap_or_enter_on_a_band_asks_for_it_again() {
        let (mut page, channel, older) = banded(Unreadable::KeyNotReceived);
        let band_y = middle_of(&page.view, 0);
        let release = |y: f32| InputEvent::PointerUp {
            x: 10.0,
            y: y.into(),
            button: PointerButton::Primary,
            ctrl: false,
        };
        button(&mut page, PointerButton::Primary, band_y);
        assert!(page.view.handle_input(&release(band_y)));
        assert_eq!(retried(&page), [(channel, older)]);

        let message_y = middle_of(&page.view, 1);
        button(&mut page, PointerButton::Primary, message_y);
        assert!(!page.view.handle_input(&release(message_y)));
        assert!(retried(&page).is_empty(), "a message is no band");

        page.view.touch_down(10.0, band_y, ambre_bus::PageTime(0.0));
        assert_eq!(
            page.view.touch_up(ambre_bus::PageTime(50.0)),
            None,
            "the tap is taken"
        );
        assert_eq!(retried(&page), [(channel, older)]);

        assert!(
            !key(&mut page, "Enter", false, ambre_bus::PageTime(0.0)),
            "no focus yet"
        );
        while page.view.focused() != Some(RowKey::Band(older)) {
            assert!(key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0)));
            frame(&mut page.view);
        }
        let _ = reads(&page);
        assert!(key(&mut page, "Enter", false, ambre_bus::PageTime(0.0)));
        assert_eq!(retried(&page), [(channel, older)]);
    }

    /// A segment before the history boundary stays closed: its band asks for
    /// nothing, and the tap passes on (`MURMURE-513`).
    #[test]
    fn a_band_before_the_history_boundary_asks_for_nothing() {
        let (mut page, _, _) = banded(Unreadable::BeforeHistoryBoundary);
        let band_y = middle_of(&page.view, 0);
        page.view.touch_down(10.0, band_y, ambre_bus::PageTime(0.0));
        assert_eq!(
            page.view.touch_up(ambre_bus::PageTime(50.0)),
            Some((10.0, band_y))
        );
        assert!(retried(&page).is_empty());
    }

    /// A right-click on a message hands the page that message, where it lies
    /// and its text (`MURMURE-422`). One off any message hands none, and a
    /// left click hands nothing. No command leaves (`MURMURE-424`).
    #[test]
    fn a_right_click_hands_the_page_its_message_and_sends_nothing() {
        let mut page = page();
        let channel = ChannelId::new();
        let ada = UserId::new();
        let segment = SegmentId::new();
        let messages = vec![
            message(ada, ambre_bus::Timestamp(0), "a"),
            message(ada, ambre_bus::Timestamp(1), "b"),
        ];
        let second = messages[1].id;
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        page.pane.set(MurmurePane::channel(channel));
        frame(&mut page.view);
        let _ = reads(&page);

        let y = middle_of(&page.view, 1);
        assert!(!button(&mut page, PointerButton::Secondary, y));
        let target = MessageTarget {
            channel,
            at: MessageAt::Segment(segment),
            message: second,
            author: ada,
            text: "b".to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
            notice: false,
            deleted: false,
            pressed: None,
            canvas: CHANNEL_CANVAS,
            from_thread: None,
        };
        assert_eq!(page.acted.borrow().as_slice(), [Some(target)]);
        // The list rests on its latest row: the top of the region is empty.
        let above = shown(&page.view).rows.first().map_or(0.0, |r| r.1) - 1.0;
        assert!(above > 0.0, "room above the first row");
        button(&mut page, PointerButton::Secondary, above);
        assert_eq!(page.acted.borrow().last(), Some(&None), "off any message");
        button(&mut page, PointerButton::Primary, y);
        assert_eq!(page.acted.borrow().len(), 2, "a left click opens nothing");
        assert!(page.commands.try_recv().is_err(), "nothing was sent");
    }

    /// A finger held still on a reply hands the page that reply at the
    /// point it held, in the thread's canvas (`MURMURE-423`). The lift is no
    /// tap, and no command leaves (`MURMURE-424`).
    #[test]
    fn a_long_press_hands_the_page_its_message_where_it_held() {
        let mut page = page();
        let channel = ChannelId::new();
        let ada = UserId::new();
        let root = message(ada, ambre_bus::Timestamp(0), "root");
        let segment = SegmentId::new();
        let messages = vec![root.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let reply = message(ada, ambre_bus::Timestamp(1), "one");
        let ev = MurmureEvent::ThreadMessages {
            root: root.id,
            thread: DocumentId::new(),
            messages: vec![reply.clone()],
        };
        send(&page, channel, ev);
        let mut thread_view = page.view.region(300.0, 400.0);
        page.pane.set(MurmurePane::Thread {
            channel,
            root: MessageRef {
                segment,
                message: root.id,
            },
        });
        frame(&mut page.view);
        frame(&mut thread_view);
        let _ = reads(&page);

        let y = middle_of(&thread_view, 0);
        thread_view.touch_down(10.0, y, ambre_bus::PageTime(0.0));
        frame_at(&mut thread_view, ambre_bus::PageTime(LONG_PRESS_MS - 1.0));
        assert!(page.acted.borrow().is_empty(), "not yet held");
        frame_at(&mut thread_view, ambre_bus::PageTime(LONG_PRESS_MS));
        let target = MessageTarget {
            channel,
            at: MessageAt::Thread(root.id),
            message: reply.id,
            author: ada,
            text: "one".to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
            notice: false,
            deleted: false,
            pressed: Some((10.0, y)),
            canvas: THREAD_CANVAS,
            from_thread: None,
        };
        assert_eq!(page.acted.borrow().as_slice(), [Some(target)]);
        assert_eq!(
            thread_view.touch_up(ambre_bus::PageTime(LONG_PRESS_MS + 100.0)),
            None,
            "no tap"
        );
        frame_at(&mut thread_view, ambre_bus::PageTime(LONG_PRESS_MS + 200.0));
        assert_eq!(page.acted.borrow().len(), 1, "said once");
        assert!(page.commands.try_recv().is_err(), "nothing was sent");
    }

    /// The ContextMenu key and Shift+F10 hand the page the focused message,
    /// held at the foot of its row (`MURMURE-511`). With no focus they hand
    /// none, a plain F10 stays the browser's, and no command leaves.
    #[test]
    fn the_menu_key_hands_the_page_the_focused_message_at_its_row() {
        let (mut page, channel, rows) = long_channel(400);
        frame(&mut page.view);
        assert!(key(
            &mut page,
            "ContextMenu",
            false,
            ambre_bus::PageTime(10.0)
        ));
        assert_eq!(page.acted.borrow().as_slice(), [None], "no focus yet");

        assert!(key(&mut page, "ArrowUp", false, ambre_bus::PageTime(20.0)));
        frame(&mut page.view);
        let (_, bottom) = shown(&page.view).focus.expect("the focus on screen");
        // The paints at the bottom marked the latest message read.
        while page.commands.try_recv().is_ok() {}
        assert!(key(
            &mut page,
            "ContextMenu",
            false,
            ambre_bus::PageTime(30.0)
        ));
        let shift_f10 = InputEvent::Key {
            key: "F10".to_owned(),
            ctrl: false,
            alt: false,
            shift: true,
            meta: false,
        };
        assert!(page.view.handle_input(&shift_f10));
        assert!(
            !key(&mut page, "F10", false, ambre_bus::PageTime(40.0)),
            "a plain F10 is the browser's"
        );
        let acted = page.acted.borrow();
        assert_eq!(acted.len(), 3);
        for target in acted.iter().skip(1) {
            let target = target.as_ref().expect("the focused message");
            assert_eq!(target.channel, channel);
            assert_eq!(target.message, rows[399].id);
            assert_eq!(target.pressed, Some((0.0, bottom)));
            assert_eq!(target.canvas, CHANNEL_CANVAS);
        }
        assert!(page.commands.try_recv().is_err(), "nothing was sent");
    }

    /// The pane that names a message rests the list on it, a quarter down,
    /// once the message is in the list (`MURMURE-150`).
    #[test]
    fn a_pane_at_a_message_rests_the_list_on_it() {
        let mut page = page();
        let channel = ChannelId::new();
        let messages = segment(&page, channel, 1_000, 200);
        let named = messages.get(20).expect("a message").id;
        let at = MessageRef {
            segment: SegmentId::new(),
            message: named,
        };
        page.pane.set(MurmurePane::Channel {
            channel,
            at: Some(at),
        });
        assert!(frame(&mut page.view));
        // The list rests on its first row on screen, a few rows above the
        // one named, a quarter down the viewport of 400.
        let Some((RowKey::Message(top), _)) = page.view.resting_on() else {
            panic!("resting on a message");
        };
        let at = messages.iter().position(|m| m.id == top);
        assert!(at.is_some_and(|i| (14..20).contains(&i)), "{at:?}");
        assert!(!shown(&page.view).latest_shown);

        let later = message(UserId::new(), ambre_bus::Timestamp(9_000), "later");
        let at = MessageRef {
            segment: SegmentId::new(),
            message: later.id,
        };
        page.pane.set(MurmurePane::Channel {
            channel,
            at: Some(at),
        });
        assert!(frame(&mut page.view));
        assert_eq!(page.view.resting_on(), None, "waiting for its message");
        let segment = SegmentId::new();
        let messages = vec![later.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        assert!(frame(&mut page.view));
        assert!(shown(&page.view).latest_shown, "it came as the latest");
    }

    /// The region follows the pointer and consumes nothing.
    #[test]
    fn the_region_follows_the_pointer_and_consumes_nothing() {
        let mut page = page();
        assert_eq!(page.view.hover(), None);
        let ev = InputEvent::PointerMove { x: 3.0, y: 4.0 };
        assert!(!page.view.handle_input(&ev));
        assert_eq!(page.view.hover(), Some(Point::new(3.0, 4.0)));
        assert_eq!(page.view.size(), (300.0, 400.0));
    }

    /// A page on `channel`, painted once so the composer's band exists.
    fn composing(channel: ChannelId) -> Page {
        let mut page = page();
        list(&page, channel, &["a"]);
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        page
    }

    /// A primary press in the composer's field, near the region's bottom,
    /// above the box of a thread.
    fn press_composer(page: &mut Page) -> bool {
        let y = if page.view.composer().on_also(395.0) {
            395.0 - ALSO_PX
        } else {
            395.0
        };
        press_at(page, y)
    }

    /// A primary press and its release in the composer's band at `y`.
    fn press_at(page: &mut Page, y: f32) -> bool {
        let press = InputEvent::PointerDown {
            x: 40.0,
            y: y.into(),
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        };
        let consumed = page.view.handle_input(&press);
        let up = InputEvent::PointerUp {
            x: 40.0,
            y: y.into(),
            button: PointerButton::Primary,
            ctrl: false,
        };
        page.view.handle_input(&up);
        consumed
    }

    fn type_text(page: &mut Page, text: &str) -> bool {
        let ev = InputEvent::Text {
            text: text.to_owned(),
        };
        page.view.handle_input(&ev)
    }

    fn shift_key(page: &mut Page, key: &str) -> bool {
        let ev = InputEvent::Key {
            key: key.to_owned(),
            ctrl: false,
            alt: false,
            shift: true,
            meta: false,
        };
        page.view.handle_input(&ev)
    }

    /// The messages sent since the last call, as text and time.
    fn sent(page: &Page) -> Vec<(String, u64)> {
        let mut sent = Vec::new();
        while let Ok(command) = page.commands.try_recv() {
            if let LogicCommand::Channel {
                command:
                    MurmureCommand::SendMessage {
                        draft,
                        now,
                        confirmed,
                        ..
                    },
                ..
            } = command
            {
                assert!(!confirmed, "a plain send asks no confirmation");
                assert!(draft.mentions.is_empty());
                sent.push((draft.text, now.0));
            }
        }
        sent
    }

    /// Enter sends the draft at the clock's time and empties it
    /// (`MURMURE-199`). Shift+Enter breaks the line and sends nothing
    /// (`MURMURE-200`).
    #[test]
    fn enter_sends_the_draft_and_shift_enter_breaks_the_line() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        assert!(press_composer(&mut page));
        assert!(type_text(&mut page, "a"));
        assert!(shift_key(&mut page, "Enter"));
        assert!(type_text(&mut page, "b"));
        frame_with_fonts(&mut page.view);
        assert!(sent(&page).is_empty(), "Shift+Enter sends nothing");
        assert_eq!(page.view.composer().draft(channel).as_deref(), Some("a\nb"));

        // The page's clock restarts at every load: the message is dated by the
        // wall clock, never by it.
        page.wall.set(Some(Timestamp(1_700_000_000_000)));
        assert!(key(&mut page, "Enter", false, ambre_bus::PageTime(1_234.0)));
        frame_with_fonts(&mut page.view);
        assert_eq!(sent(&page), vec![("a\nb".to_owned(), 1_700_000_000_000)]);
        assert_eq!(
            page.view.composer().draft(channel),
            None,
            "the draft is gone"
        );
    }

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// Drop `name` on the canvas, paint, and return the upload it asked.
    fn drop_file(page: &mut Page, channel: ChannelId, name: &str) -> RequestId {
        page.dropped.borrow_mut().push(PickedFile {
            name: name.to_owned(),
            media_type: MediaType(String::new()),
            data: vec![7; 3],
        });
        frame_with_fonts(&mut page.view);
        let mut uploads = Vec::new();
        while let Ok(command) = page.commands.try_recv() {
            if let LogicCommand::Channel {
                channel_id,
                command: MurmureCommand::UploadFile { id, data },
            } = command
            {
                assert_eq!((channel_id, data), (channel, vec![7; 3]));
                uploads.push(id);
            }
        }
        let [id] = uploads.as_slice() else {
            panic!("one upload per file, got {uploads:?}");
        };
        *id
    }

    /// The files of each message sent since the last call.
    fn sent_files(page: &Page) -> Vec<Vec<Attachment>> {
        std::iter::from_fn(|| page.commands.try_recv().ok())
            .filter_map(|command| match command {
                LogicCommand::Channel {
                    command: MurmureCommand::SendMessage { attachments, .. },
                    ..
                } => Some(attachments),
                _ => None,
            })
            .collect()
    }

    /// `MURMURE-132`: a dropped file uploads before its message leaves. The
    /// draft holds while it uploads, and carries it once the relay answered.
    #[test]
    fn a_dropped_file_uploads_and_its_message_carries_it() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        let id = drop_file(&mut page, channel, "a.png");
        assert!(press_composer(&mut page));
        assert!(type_text(&mut page, "see"));
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert!(sent_files(&page).is_empty(), "the upload holds the send");

        let evt = RestEvent::MediaUploaded {
            document_id: DocumentId::new(),
            content_hash: ambre_core::ContentHash(HASH.to_owned()),
        };
        page.events
            .try_send(UiEvent::Rest { id, evt })
            .expect("room");
        frame_with_fonts(&mut page.view);
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let sent = sent_files(&page);
        assert_eq!(sent.len(), 1);
        let [file] = sent[0].as_slice() else {
            panic!("one file, got {:?}", sent[0]);
        };
        assert_eq!((file.name.as_str(), file.size), ("a.png", ByteSize(3)));
        assert_eq!(file.media_type.0, "application/octet-stream");
        assert_eq!(file.media_ref.as_str(), HASH);
    }

    /// `MURMURE-133`: a failed upload adds nothing to the message, which
    /// then leaves on its text alone, and the user is warned by the file's
    /// name.
    #[test]
    fn a_failed_upload_adds_nothing_to_the_message() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        let id = drop_file(&mut page, channel, "a.png");
        let failed = UiEvent::ApiError {
            id: Some(id),
            request: ambre_bus::ApiRequest::UploadMedia,
            failure: ambre_bus::ApiFailure::Relay {
                status: ambre_bus::HttpStatus(500),
                code: None,
            },
            message: "down".to_owned(),
        };
        page.events.try_send(failed).expect("room");
        assert!(press_composer(&mut page));
        assert!(type_text(&mut page, "see"));
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(sent_files(&page), vec![Vec::new()]);
        assert_eq!(*page.warned.borrow(), ["a.png could not be attached"]);
    }

    /// A press on a file of the draft takes it off, so the draft sends
    /// nothing of it.
    #[test]
    fn a_press_on_a_file_of_the_draft_takes_it_off() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        let id = drop_file(&mut page, channel, "a.png");
        let evt = RestEvent::MediaUploaded {
            document_id: DocumentId::new(),
            content_hash: ambre_core::ContentHash(HASH.to_owned()),
        };
        page.events
            .try_send(UiEvent::Rest { id, evt })
            .expect("room");
        frame_with_fonts(&mut page.view);
        let hit = (0..300)
            .flat_map(|x| (0..400).map(move |y| (x as f32, y as f32)))
            .find(|&(x, y)| page.view.composer().file_at(x, y).is_some())
            .expect("the file shows in the composer");
        let press = InputEvent::PointerDown {
            x: hit.0.into(),
            y: hit.1.into(),
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        };
        page.view.handle_input(&press);
        frame_with_fonts(&mut page.view);
        assert!(page.view.composer().file_at(hit.0, hit.1).is_none());
        assert!(press_composer(&mut page));
        assert!(type_text(&mut page, "see"));
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(sent_files(&page), vec![Vec::new()]);
    }

    /// A clicked file is asked of the relay under its channel. Its answer
    /// goes to the page, which opens or saves it.
    #[test]
    fn a_file_clicked_is_fetched_and_handed_to_the_page() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        let file = Attachment {
            media_ref: MediaRef::parse(HASH).expect("an address"),
            name: "a.png".to_owned(),
            media_type: MediaType("image/png".to_owned()),
            size: ByteSize(3),
        };
        let pane = MurmurePane::Channel { channel, at: None };
        while page.commands.try_recv().is_ok() {}
        page.view.fetch_file(pane, file.clone());
        let Ok(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::FetchFile { id, media_ref },
        }) = page.commands.try_recv()
        else {
            panic!("a fetch of the file");
        };
        assert_eq!((channel_id, media_ref.as_str()), (channel, HASH));
        let evt = RestEvent::MediaDownloaded {
            document_id: DocumentId::new(),
            content_hash: ambre_core::ContentHash(HASH.to_owned()),
            data: vec![1, 2, 3].into(),
        };
        page.events
            .try_send(UiEvent::Rest { id, evt })
            .expect("room");
        frame_with_fonts(&mut page.view);
        assert_eq!(*page.saved.borrow(), vec![(file, vec![1, 2, 3])]);
    }

    /// The pictures a paint asked the Logic thread for since the last call.
    fn pictures_asked(page: &Page) -> usize {
        std::iter::from_fn(|| page.commands.try_recv().ok())
            .filter(|command| {
                matches!(
                    command,
                    LogicCommand::Channel {
                        command: MurmureCommand::ShowPicture { .. },
                        ..
                    }
                )
            })
            .count()
    }

    /// A message of `texts.len()` rows whose first carries an animated image.
    fn gif_list(page: &Page, channel: ChannelId, texts: &[&str]) -> Attachment {
        let file = Attachment {
            media_ref: MediaRef::parse(HASH).expect("an address"),
            name: "a.gif".to_owned(),
            media_type: MediaType("image/gif".to_owned()),
            size: ByteSize(3),
        };
        let ada = UserId::new();
        let mut messages: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(i, text)| message(ada, ambre_bus::Timestamp(i as u64), text))
            .collect();
        messages[0].attachments = vec![file.clone()];
        let segment = SegmentId::new();
        send(page, channel, MurmureEvent::Messages { segment, messages });
        file
    }

    /// `MURMURE-136`: a picture on screen is asked once. Once it lands, the
    /// frame loop repaints when its next frame is due, and not before.
    #[test]
    fn a_picture_on_screen_is_asked_once_and_plays_its_frames() {
        use ambre_bus::{Picture, PictureFrame};
        let channel = ChannelId::new();
        let mut page = page();
        let file = gif_list(&page, channel, &["see"]);
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        assert_eq!(pictures_asked(&page), 1);
        frame_with_fonts(&mut page.view);
        assert_eq!(pictures_asked(&page), 0, "asked once");

        let frame = || PictureFrame {
            pixels: vec![0; 2 * 4],
            delay_ms: ambre_core::Millis(100),
        };
        let gif = Picture {
            width: 2,
            height: 1,
            frames: vec![frame(), frame()],
        };
        let picture = Some(gif);
        let media_ref = file.media_ref.clone();
        send(&page, channel, MurmureEvent::Picture { media_ref, picture });
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.last_painted().and_then(|p| p.next_frame),
            Some(ambre_core::PageTime(100.0))
        );
        assert!(
            !frame_at(&mut page.view, ambre_bus::PageTime(50.0)),
            "no frame is due yet"
        );
        page.clock.set(ambre_bus::PageTime(100.0));
        assert!(
            frame_at(&mut page.view, ambre_bus::PageTime(100.0)),
            "the next frame is due"
        );
    }

    /// `MURMURE-130`: under reduced motion a picture holds its first frame,
    /// and the frame loop never repaints for it.
    #[test]
    fn under_reduced_motion_a_picture_never_asks_a_frame() {
        use ambre_bus::{Picture, PictureFrame};
        let channel = ChannelId::new();
        let mut page = page();
        page.still.set(true);
        let file = gif_list(&page, channel, &["see"]);
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        let frame = || PictureFrame {
            pixels: vec![0; 4],
            delay_ms: ambre_core::Millis(100),
        };
        let picture = Some(Picture {
            width: 1,
            height: 1,
            frames: vec![frame(), frame()],
        });
        let media_ref = file.media_ref.clone();
        send(&page, channel, MurmureEvent::Picture { media_ref, picture });
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.last_painted().and_then(|p| p.next_frame), None);
        assert!(!frame_at(&mut page.view, ambre_bus::PageTime(1000.0)));
    }

    /// `MURMURE-131`: a picture scrolled off screen is not painted, so it is
    /// neither fetched nor played.
    #[test]
    fn a_picture_off_screen_is_not_asked() {
        let channel = ChannelId::new();
        let mut page = page();
        let texts = vec!["row"; 60];
        gif_list(&page, channel, &texts);
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        assert_eq!(pictures_asked(&page), 0);
        assert!(
            page.view
                .last_painted()
                .is_some_and(|p| p.pictures.is_empty())
        );
    }

    /// An empty draft, or one of white space alone, sends nothing and
    /// stays (`MURMURE-201`).
    #[test]
    fn an_empty_or_blank_draft_sends_nothing() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        assert!(press_composer(&mut page));
        assert!(key(&mut page, "Enter", false, ambre_bus::PageTime(0.0)));
        frame_with_fonts(&mut page.view);
        assert!(sent(&page).is_empty());
        type_text(&mut page, "  ");
        shift_key(&mut page, "Enter");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert!(sent(&page).is_empty(), "white space is not a message");
        assert_eq!(page.view.composer().draft(channel).as_deref(), Some("  \n"));
    }

    /// Each channel keeps its own draft: a send in one leaves the other's
    /// draft as it was (`MURMURE-195`, `MURMURE-202`).
    #[test]
    fn each_channel_keeps_its_own_draft() {
        let (a, b) = (ChannelId::new(), ChannelId::new());
        let mut page = composing(a);
        press_composer(&mut page);
        type_text(&mut page, "for a");
        list(&page, b, &["b"]);
        page.pane.set(MurmurePane::Channel {
            channel: b,
            at: None,
        });
        frame_with_fonts(&mut page.view);
        press_composer(&mut page);
        type_text(&mut page, "for b");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(sent(&page), vec![("for b".to_owned(), 0)]);
        assert_eq!(page.view.composer().draft(b), None);
        // A's keys wait for the next paint of A, and land there.
        page.pane.set(MurmurePane::Channel {
            channel: a,
            at: None,
        });
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().draft(a).as_deref(), Some("for a"));
        assert!(sent(&page).is_empty(), "A's draft was not sent");
    }

    /// The band holds a field and its text, no toolbar, and the list
    /// stands above it (`MURMURE-196`).
    #[test]
    fn the_band_holds_a_field_and_no_toolbar() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        let band = page.view.composer().band();
        assert!(band > 0.0, "a channel region shows the composer");
        assert_eq!(page.view.list_height(), 400.0 - band);
        page.view.update();
        page.view.plan_repaint(ambre_bus::PageTime(0.0), false);
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let mut surface = RecordingSurface::new();
        page.view
            .paint_content(&mut surface, &mut font_cx, &mut cache, None);
        let top = 400.0 - band;
        let in_band = surface.ops().iter().filter(|op| match op {
            ambre_draw::testing::DrawOp::FillRect { rect, .. }
            | ambre_draw::testing::DrawOp::StrokeRect { rect, .. } => rect.y0 >= f64::from(top),
            _ => false,
        });
        // The band's fill, the field's fill and the field's outline.
        assert_eq!(in_band.count(), 3, "no button and no toolbar");

        page.pane.set(MurmurePane::ChannelList);
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().band(), 0.0, "no channel, no composer");
    }

    /// A press in the band gives the pipe the keyboard and is consumed.
    /// The keys then go to the draft, never to the list, until Escape
    /// (`MURMURE-430`).
    #[test]
    fn the_composer_takes_the_pipe_until_escape() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        assert_eq!(page.view.reads.pipe.focus_requests(), 0);
        assert!(press_composer(&mut page), "the press is the composer's");
        assert_eq!(page.view.reads.pipe.focus_requests(), 1);
        assert!(page.view.composer().focused());
        type_text(&mut page, "ab");
        assert!(key(&mut page, "ArrowLeft", false, ambre_bus::PageTime(0.0)));
        assert!(shift_key(&mut page, "ArrowLeft"));
        assert!(
            key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0)),
            "the caret takes it"
        );
        frame_with_fonts(&mut page.view);
        assert_eq!(
            *page.said.borrow(),
            ["ab"],
            "the composer said its line, and the list announced nothing"
        );
        assert!(
            !key(&mut page, "F5", false, ambre_bus::PageTime(0.0)),
            "a key it does not use stays the page's"
        );

        assert!(key(&mut page, "Escape", false, ambre_bus::PageTime(0.0)));
        assert!(!page.view.composer().focused());
        assert!(
            key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0)),
            "the list takes it again"
        );
        frame_with_fonts(&mut page.view);
        assert!(!page.said.borrow().is_empty(), "the list moved its focus");
    }

    /// A paste lands in the draft, and a copy or a cut reads the draft's
    /// selection while the composer holds the focus (`MURMURE-430`).
    #[test]
    fn paste_copy_and_cut_go_through_the_draft() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        page.view.paste("hello");
        assert!(
            key(&mut page, "a", true, ambre_bus::PageTime(0.0)),
            "Ctrl+A selects the draft"
        );
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.clipboard(ClipboardWrite::Copy).as_deref(),
            Some("hello")
        );
        assert_eq!(
            page.view.clipboard(ClipboardWrite::Cut).as_deref(),
            Some("hello")
        );
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().draft(channel).as_deref(), Some(""));

        key(&mut page, "Escape", false, ambre_bus::PageTime(0.0));
        assert_eq!(
            page.view.clipboard(ClipboardWrite::Cut),
            None,
            "the list's text stays"
        );
    }

    /// A `Ctrl` chord, with Shift when `shift`.
    fn chord(page: &mut Page, key: &str, shift: bool) -> bool {
        let ev = InputEvent::Key {
            key: key.to_owned(),
            ctrl: true,
            alt: false,
            shift,
            meta: false,
        };
        page.view.handle_input(&ev)
    }

    /// A `Cmd` chord, as macOS reports it: the command key, no control.
    fn command(page: &mut Page, key: &str) -> bool {
        let ev = InputEvent::Key {
            key: key.to_owned(),
            ctrl: false,
            alt: false,
            shift: false,
            meta: true,
        };
        page.view.handle_input(&ev)
    }

    /// `UI-THREAD-028`: on macOS the composer's chords are the command key's.
    /// `Cmd+B` styles the selection, `Cmd+Z` takes a word back, and `Cmd+;`
    /// opens the picker, as `Ctrl` does elsewhere.
    #[test]
    fn the_command_key_holds_the_composer_chords() {
        use ambre_bus::TextStyle::Bold;
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "hi yo");
        for _ in 0..2 {
            shift_key(&mut page, "ArrowLeft");
        }
        frame_with_fonts(&mut page.view);
        assert!(command(&mut page, "b"), "the composer takes Cmd+B");
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().styles(channel), [styled(Bold, 3..5)]);

        key(&mut page, "End", false, ambre_bus::PageTime(0.0));
        assert!(command(&mut page, "z"), "the composer takes Cmd+Z");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("hi "));

        assert!(command(&mut page, ";"), "the composer takes Cmd+;");
        assert_eq!(*page.asked_emoji.borrow(), [Region::Channel]);
    }

    /// Type `text` one key per frame, as a member does.
    fn type_slowly(page: &mut Page, text: &str) {
        for c in text.chars() {
            type_text(page, &c.to_string());
            frame_with_fonts(&mut page.view);
        }
    }

    /// The draft of a channel or a thread once the keys before it landed.
    fn painted(page: &mut Page, of: impl Into<DraftOf>) -> Option<String> {
        frame_with_fonts(&mut page.view);
        page.view.composer().draft(of)
    }

    /// `Ctrl+Z` takes back a word at a time, and `Ctrl+Shift+Z` or `Ctrl+Y`
    /// brings it again. A run of deletions is one step, a paste is one, and
    /// an edit drops what a redo held (`MURMURE-433`).
    #[test]
    fn undo_and_redo_walk_the_draft_a_word_at_a_time() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "hi yo");
        assert!(chord(&mut page, "z", false), "the composer takes the chord");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("hi "));
        chord(&mut page, "z", false);
        assert_eq!(painted(&mut page, channel).as_deref(), Some(""));
        chord(&mut page, "z", false);
        assert_eq!(
            painted(&mut page, channel).as_deref(),
            Some(""),
            "nothing left to undo"
        );
        assert!(chord(&mut page, "Z", true));
        assert_eq!(painted(&mut page, channel).as_deref(), Some("hi "));
        assert!(chord(&mut page, "y", false));
        assert_eq!(painted(&mut page, channel).as_deref(), Some("hi yo"));

        for _ in 0..2 {
            key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
            frame_with_fonts(&mut page.view);
        }
        assert_eq!(painted(&mut page, channel).as_deref(), Some("hi "));
        chord(&mut page, "z", false);
        assert_eq!(
            painted(&mut page, channel).as_deref(),
            Some("hi yo"),
            "two deletions are one step"
        );

        page.view.paste("there");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("hi yothere"));
        chord(&mut page, "z", false);
        assert_eq!(painted(&mut page, channel).as_deref(), Some("hi yo"));
        type_slowly(&mut page, "x");
        chord(&mut page, "Z", true);
        assert_eq!(
            painted(&mut page, channel).as_deref(),
            Some("hi yox"),
            "the edit dropped the redo"
        );
    }

    /// A send ends the history: an undo after it brings back no message
    /// sent, and sends nothing (`MURMURE-434`).
    #[test]
    fn an_undo_after_a_send_reaches_no_message() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "hello");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(sent(&page).len(), 1);
        chord(&mut page, "z", false);
        assert_eq!(painted(&mut page, channel).as_deref(), Some(""));
        chord(&mut page, "Z", true);
        assert_eq!(painted(&mut page, channel).as_deref(), Some(""));
        assert!(sent(&page).is_empty(), "no undo reaches the sent message");
    }

    /// The edits sent since the last call, and how many new messages.
    fn edits(page: &Page) -> (Vec<(MessageAt, MessageId, ambre_bus::MessageDraft)>, usize) {
        let (mut edits, mut messages) = (Vec::new(), 0);
        while let Ok(command) = page.commands.try_recv() {
            match command {
                LogicCommand::Channel {
                    command:
                        MurmureCommand::EditMessage {
                            at, message, draft, ..
                        },
                    ..
                } => edits.push((at, message, draft)),
                LogicCommand::Channel {
                    command: MurmureCommand::SendMessage { .. },
                    ..
                } => messages += 1,
                _ => {}
            }
        }
        (edits, messages)
    }

    /// An edit of `message`, reading `text`, as the actions of a message
    /// hand it to the page.
    fn ask_edit(page: &Page, channel: ChannelId, message: MessageId, text: &str) -> MessageAt {
        let at = MessageAt::Segment(SegmentId::new());
        let ada = ambre_bus::Mention {
            target: ambre_bus::MentionTarget::User(UserId::new()),
            range: CharSpan::from_usize_range(3..7),
        };
        *page.editing.borrow_mut() = Some(PendingEdit {
            channel,
            at,
            message,
            text: text.to_owned(),
            mentions: vec![ada],
            styles: Vec::new(),
        });
        at
    }

    /// The edit loads the message into the composer, with the draft put
    /// aside. Enter sends an edit of that message and no new one, and the
    /// draft comes back. A blank edit sends nothing, and an unchanged one
    /// ends sending nothing (`MURMURE-203`).
    #[test]
    fn an_edit_loads_the_message_and_sends_an_edit_never_a_new_one() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "draft");
        let message = MessageId::new();
        let at = ask_edit(&page, channel, message, "hi @ada there");
        assert_eq!(
            painted(&mut page, channel).as_deref(),
            Some("hi @ada there")
        );
        assert!(page.view.composer().editing(channel));
        for _ in 0.."there".len() {
            key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        }
        type_text(&mut page, "you");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        assert_eq!(painted(&mut page, channel).as_deref(), Some("draft"));
        let (sent_edits, messages) = edits(&page);
        assert_eq!(messages, 0, "an edit sends no new message");
        let [(edit_at, edited, draft)] = &sent_edits[..] else {
            panic!("one edit, got {sent_edits:?}");
        };
        assert_eq!((*edit_at, *edited), (at, message));
        assert_eq!(draft.text, "hi @ada you");
        assert_eq!(draft.mentions.len(), 1, "the mention the edit left stays");
        assert!(!page.view.composer().editing(channel));

        ask_edit(&page, channel, message, "hi");
        frame_with_fonts(&mut page.view);
        key(&mut page, "a", true, ambre_bus::PageTime(0.0));
        key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert!(page.view.composer().editing(channel), "a blank edit stays");
        type_text(&mut page, "hi");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        assert_eq!(painted(&mut page, channel).as_deref(), Some("draft"));
        assert_eq!(
            edits(&page),
            (Vec::new(), 0),
            "nothing changed, nothing sent"
        );
    }

    /// Escape ends an edit and sends nothing, and the draft comes back. The
    /// composer keeps the keys until a second Escape (`MURMURE-204`).
    #[test]
    fn escape_cancels_an_edit_and_leaves_the_message() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "draft");
        ask_edit(&page, channel, MessageId::new(), "hello");
        frame_with_fonts(&mut page.view);
        type_text(&mut page, " world");
        frame_with_fonts(&mut page.view);
        assert!(key(&mut page, "Escape", false, ambre_bus::PageTime(0.0)));
        assert_eq!(painted(&mut page, channel).as_deref(), Some("draft"));
        assert!(!page.view.composer().editing(channel));
        assert!(
            page.view.composer().focused(),
            "the composer keeps the keys"
        );
        assert_eq!(edits(&page), (Vec::new(), 0), "nothing sent");
        key(&mut page, "Escape", false, ambre_bus::PageTime(0.0));
        assert!(!page.view.composer().focused());
    }

    /// Shift+Page Up brings the unread line into view, and moves no read
    /// cursor. With no line, the list stays (`MURMURE-175`).
    #[test]
    fn shift_page_up_brings_the_list_to_the_unread_line() {
        let (mut page, channel, rows) = long_channel(400);
        let first = rows[100].id;
        send(
            &page,
            channel,
            MurmureEvent::FirstUnread {
                message: Some(first),
            },
        );
        frame(&mut page.view);
        assert!(!shown(&page.view).line_shown, "the line is far up");
        assert_eq!(reads(&page).0, 1);
        assert!(shift_key(&mut page, "PageUp"), "the list takes the key");
        frame(&mut page.view);
        let painted = shown(&page.view);
        assert!(painted.line_shown && !painted.latest_shown);
        assert_eq!(reads(&page), (0, 0), "a jump reads nothing");

        let (mut quiet, _, _) = long_channel(400);
        frame(&mut quiet.view);
        assert!(shift_key(&mut quiet, "PageUp"));
        frame(&mut quiet.view);
        assert_eq!(quiet.view.resting_on(), None, "no line, no jump");
    }

    /// Escape in the list marks the channel read, high up as well. In the
    /// composer it gives the keys back, and marks nothing (`MURMURE-522`).
    #[test]
    fn escape_in_the_list_marks_the_channel_read() {
        let (mut page, _, _) = long_channel(400);
        frame(&mut page.view);
        assert!(key(&mut page, "PageUp", false, ambre_bus::PageTime(10.0)));
        frame_at(&mut page.view, ambre_bus::PageTime(10.0));
        assert_eq!(reads(&page).0, 1, "the first paint read the latest");
        assert!(key(&mut page, "Escape", false, ambre_bus::PageTime(20.0)));
        assert_eq!(reads(&page).0, 1, "Escape marks the channel read");

        let channel = ChannelId::new();
        let mut composing = composing(channel);
        reads(&composing);
        press_composer(&mut composing);
        assert!(key(
            &mut composing,
            "Escape",
            false,
            ambre_bus::PageTime(0.0)
        ));
        assert!(!composing.view.composer().focused());
        assert_eq!(
            reads(&composing),
            (0, 0),
            "the composer's Escape reads nothing"
        );

        let thread_view = composing.view.region(300.0, 400.0);
        let mut thread = Page {
            view: thread_view,
            ..composing
        };
        assert!(
            !key(&mut thread, "Escape", false, ambre_bus::PageTime(0.0)),
            "the thread leaves it"
        );
    }

    /// Messages of `me` and of another member, in one segment of `channel`
    /// on screen: the member's own, one of theirs deleted, then another's.
    fn mixed_channel(channel: ChannelId) -> (Page, SegmentId, MessageId) {
        let mut page = page();
        let (me, bob) = (UserId::new(), UserId::new());
        page.me.set(Some(me));
        let own = message(me, ambre_bus::Timestamp(0), "mine");
        let deleted = MessageView {
            deleted: true,
            text: String::new(),
            ..message(me, ambre_bus::Timestamp(1), "gone")
        };
        let messages = vec![
            own.clone(),
            deleted,
            message(bob, ambre_bus::Timestamp(2), "theirs"),
        ];
        let segment = SegmentId::new();
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        page.pane.set(MurmurePane::Channel { channel, at: None });
        frame_with_fonts(&mut page.view);
        (page, segment, own.id)
    }

    /// Up in an empty composer opens the latest message of the member for
    /// edit, past a deleted one and another's. Up in a draft moves its
    /// caret (`MURMURE-178`).
    #[test]
    fn up_in_an_empty_composer_edits_the_last_own_message() {
        let channel = ChannelId::new();
        let (mut page, segment, own) = mixed_channel(channel);
        press_composer(&mut page);
        type_text(&mut page, "x");
        assert!(key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0)));
        assert_eq!(painted(&mut page, channel).as_deref(), Some("x"));
        assert!(!page.view.composer().editing(channel), "a draft keeps Up");
        // Up on the only line puts the caret at its start, as a textarea does.
        key(&mut page, "Delete", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);

        assert!(key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0)));
        assert_eq!(painted(&mut page, channel).as_deref(), Some("mine"));
        assert!(page.view.composer().editing(channel));
        type_text(&mut page, " too");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let (sent_edits, messages) = edits(&page);
        let [(at, message, draft)] = &sent_edits[..] else {
            panic!("one edit, got {sent_edits:?}");
        };
        assert_eq!((*at, *message), (MessageAt::Segment(segment), own));
        assert_eq!((draft.text.as_str(), messages), ("mine too", 0));
    }

    /// Up never opens a message of another member: with none of the
    /// member's own, or no member signed in, it opens nothing
    /// (`MURMURE-179`).
    #[test]
    fn up_never_opens_another_members_message() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        page.me.set(Some(UserId::new()));
        press_composer(&mut page);
        assert!(
            key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0)),
            "the caret takes it"
        );
        assert_eq!(painted(&mut page, channel).as_deref(), Some(""));
        assert!(!page.view.composer().editing(channel));

        let (mut signed_out, _, _) = mixed_channel(channel);
        signed_out.me.set(None);
        press_composer(&mut signed_out);
        key(&mut signed_out, "ArrowUp", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut signed_out.view);
        assert!(!signed_out.view.composer().editing(channel));
        assert_eq!(edits(&signed_out), (Vec::new(), 0));
    }

    /// The answer of a mark-all-read clears the unread line's source and
    /// leaves the draft as it was, unsent (`MURMURE-181`).
    #[test]
    fn marking_everything_read_keeps_the_draft() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "draft");
        sent(&page);
        send(&page, channel, MurmureEvent::FirstUnread { message: None });
        send(
            &page,
            ChannelId::new(),
            MurmureEvent::FirstUnread { message: None },
        );
        assert_eq!(painted(&mut page, channel).as_deref(), Some("draft"));
        assert!(
            page.view.composer().focused(),
            "the composer keeps the keys"
        );
        assert!(sent(&page).is_empty(), "nothing was sent");
    }

    /// A tap in the band puts the caret there and gives the pipe the
    /// keyboard; a press in the list takes it back (`MURMURE-430`).
    #[test]
    fn a_tap_in_the_band_focuses_the_composer() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        page.view.touch_down(40.0, 395.0, ambre_bus::PageTime(0.0));
        assert_eq!(
            page.view.touch_up(ambre_bus::PageTime(50.0)),
            None,
            "the tap is taken"
        );
        assert_eq!(page.view.reads.pipe.focus_requests(), 1);
        assert!(page.view.composer().focused());
        let press = InputEvent::PointerDown {
            x: 40.0,
            y: 20.0,
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        };
        page.view.handle_input(&press);
        assert!(!page.view.composer().focused(), "the list has it back");
    }

    /// The typing samples and the ticks sent since the last call.
    fn samples(page: &Page) -> (usize, usize) {
        let mut sent = (0, 0);
        while let Ok(command) = page.commands.try_recv() {
            match command {
                LogicCommand::Channel {
                    command: MurmureCommand::Typing { .. },
                    ..
                } => sent.0 += 1,
                LogicCommand::Channel {
                    command: MurmureCommand::Tick { .. },
                    ..
                } => sent.1 += 1,
                _ => {}
            }
        }
        sent
    }

    /// A key that changes the draft refreshes the typing indicator
    /// (`MURMURE-515`). A move of the caret or a selection sends nothing
    /// (`MURMURE-518`).
    #[test]
    fn a_key_that_changes_the_draft_sends_a_typing_sample() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        samples(&page);
        type_text(&mut page, "ab");
        assert_eq!(samples(&page).0, 1, "a typed key");
        key(&mut page, "ArrowLeft", false, ambre_bus::PageTime(0.0));
        shift_key(&mut page, "ArrowLeft");
        key(&mut page, "a", true, ambre_bus::PageTime(0.0));
        key(&mut page, "End", false, ambre_bus::PageTime(0.0));
        assert_eq!(
            samples(&page).0,
            0,
            "the caret and the selection type nothing"
        );
        key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        assert_eq!(samples(&page).0, 1, "a deleted character");
        shift_key(&mut page, "Enter");
        assert_eq!(samples(&page).0, 1, "a line break");
        page.view.paste("pasted");
        assert_eq!(samples(&page).0, 1, "a paste");
        key(&mut page, "a", true, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        samples(&page);
        assert!(page.view.clipboard(ClipboardWrite::Copy).is_some());
        assert_eq!(samples(&page).0, 0, "a copy changes nothing");
        assert!(page.view.clipboard(ClipboardWrite::Cut).is_some());
        assert_eq!(samples(&page).0, 1, "a cut");
    }

    /// The channel on screen ticks once per tick period, and a channel new
    /// on screen at once. The thread's region never ticks, nor a pane with
    /// no channel (`MURMURE-516`).
    #[test]
    fn the_channel_on_screen_ticks_once_per_period() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        assert_eq!(samples(&page).1, 1, "the first paint ticks");
        page.clock
            .set(ambre_bus::PageTime(TICK_MS.as_page_ms() - 1.0));
        frame(&mut page.view);
        assert_eq!(samples(&page).1, 0, "inside the period");
        page.clock.set(ambre_bus::PageTime(TICK_MS.as_page_ms()));
        frame(&mut page.view);
        assert_eq!(samples(&page).1, 1, "one period later");

        let other = ChannelId::new();
        page.pane.set(MurmurePane::Channel {
            channel: other,
            at: None,
        });
        frame(&mut page.view);
        assert_eq!(samples(&page).1, 1, "a channel new on screen ticks at once");
        let mut thread_view = page.view.region(300.0, 400.0);
        page.clock
            .set(ambre_bus::PageTime(10.0 * TICK_MS.as_page_ms()));
        frame(&mut thread_view);
        assert_eq!(samples(&page).1, 0, "the thread's region ticks nothing");
        page.pane.set(MurmurePane::ChannelList);
        frame(&mut page.view);
        assert_eq!(samples(&page).1, 0, "no channel, no tick");
    }

    /// The glyph runs painted in the strip above the field.
    fn strip_runs(page: &mut Page) -> Vec<usize> {
        page.view.update();
        page.view.plan_repaint(ambre_bus::PageTime(0.0), false);
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let mut surface = RecordingSurface::new();
        page.view
            .paint_content(&mut surface, &mut font_cx, &mut cache, None);
        let top = f64::from(400.0 - page.view.composer().band());
        let field = top + f64::from(FONT_UI + SPACE_4);
        surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                ambre_draw::testing::DrawOp::Glyphs {
                    glyph_count,
                    transform,
                    ..
                } => {
                    let y = transform.translation().y;
                    (y >= top && y < field).then_some(*glyph_count)
                }
                _ => None,
            })
            .collect()
    }

    /// The band names the members typing in the channel while their
    /// indicator lasts, and the line leaves when they stop (`MURMURE-517`).
    #[test]
    fn the_band_names_who_types() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        let band = page.view.composer().band();
        assert!(strip_runs(&mut page).is_empty(), "nobody types");
        let (ada, bob) = (UserId::new(), UserId::new());
        let watchers = |typing: Vec<UserId>| MurmureEvent::Watchers {
            watching: vec![ada, bob],
            typing,
            in_threads: Vec::new(),
        };
        send(&page, channel, watchers(vec![ada]));
        assert_eq!(strip_runs(&mut page), vec!["Ada is typing".len()]);
        assert_eq!(page.view.composer().band(), band, "the list did not move");
        send(&page, channel, watchers(vec![ada, bob]));
        assert_eq!(strip_runs(&mut page), vec!["Ada and Ada are typing".len()]);
        send(&page, channel, watchers(vec![]));
        assert!(strip_runs(&mut page).is_empty(), "the indicator expired");
        let elsewhere = ChannelId::new();
        send(&page, elsewhere, watchers(vec![ada]));
        assert!(strip_runs(&mut page).is_empty(), "another channel's typing");
        let in_a_thread = MurmureEvent::Watchers {
            watching: vec![ada],
            typing: Vec::new(),
            in_threads: vec![(MessageId::new(), vec![ada])],
        };
        send(&page, channel, in_a_thread);
        assert!(strip_runs(&mut page).is_empty(), "a thread's typing");
    }

    /// The thread's strip names who types in that thread, and nobody who
    /// types in the channel or in another thread (`MURMURE-537`).
    #[test]
    fn the_threads_strip_names_who_types_there() {
        let channel = ChannelId::new();
        let (mut page, mut channel_view, root, _) = threaded(channel);
        let (ada, bob) = (UserId::new(), UserId::new());
        let watchers = |in_threads| MurmureEvent::Watchers {
            watching: vec![ada, bob],
            typing: vec![bob],
            in_threads,
        };
        let elsewhere = vec![(MessageId::new(), vec![bob])];
        send(&page, channel, watchers(elsewhere));
        // The channel's region drains the queue into the shared lists.
        frame_with_fonts(&mut channel_view);
        assert!(strip_runs(&mut page).is_empty(), "nobody types here");
        send(&page, channel, watchers(vec![(root, vec![ada])]));
        frame_with_fonts(&mut channel_view);
        assert_eq!(strip_runs(&mut page), vec!["Ada is typing".len()]);
    }

    /// A key in a thread's composer sends a typing sample that names the
    /// thread's root, in its channel (`MURMURE-536`).
    #[test]
    fn a_key_in_a_threads_composer_names_its_root() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, root, _) = threaded(channel);
        assert!(press_composer(&mut page));
        while page.commands.try_recv().is_ok() {}
        type_text(&mut page, "a");
        let typed: Vec<_> = std::iter::from_fn(|| page.commands.try_recv().ok())
            .filter_map(|command| match command {
                LogicCommand::Channel {
                    channel_id,
                    command: MurmureCommand::Typing { thread, .. },
                } => Some((channel_id, thread)),
                _ => None,
            })
            .collect();
        assert_eq!(typed, [(channel, Some(root))]);
    }

    /// A thread under a root of `channel`, shown in the panel. Its region
    /// stands in `page.view`, and the channel's region, which drains the
    /// queue, comes back beside it. The thread holds a reply of the member,
    /// then one of another member. Answers the root and the own reply.
    fn threaded(channel: ChannelId) -> (Page, MurmureView, MessageId, MessageId) {
        let mut page = page();
        let (me, bob) = (UserId::new(), UserId::new());
        page.me.set(Some(me));
        let root = message(me, ambre_bus::Timestamp(0), "root");
        let segment = SegmentId::new();
        let messages = vec![root.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let own = message(me, ambre_bus::Timestamp(1), "mine");
        let ev = MurmureEvent::ThreadMessages {
            root: root.id,
            thread: DocumentId::new(),
            messages: vec![own.clone(), message(bob, ambre_bus::Timestamp(2), "theirs")],
        };
        send(&page, channel, ev);
        let at = MessageRef {
            segment,
            message: root.id,
        };
        page.pane.set(MurmurePane::Thread { channel, root: at });
        let thread = page.view.region(300.0, 400.0);
        let mut channel_view = std::mem::replace(&mut page.view, thread);
        frame_with_fonts(&mut channel_view);
        frame_with_fonts(&mut page.view);
        (page, channel_view, root.id, own.id)
    }

    /// The replies sent since the last call, as root, text and time. A
    /// message sent to the channel fails the test.
    fn replies(page: &Page) -> Vec<(MessageId, String, u64)> {
        let mut replies = Vec::new();
        while let Ok(command) = page.commands.try_recv() {
            let LogicCommand::Channel { command, .. } = command else {
                continue;
            };
            match command {
                MurmureCommand::Reply {
                    root,
                    draft,
                    also_in_channel,
                    now,
                    confirmed,
                } => {
                    assert!(
                        also_in_channel == ReplyEcho::ThreadOnly,
                        "the reply stays in its thread"
                    );
                    assert!(!confirmed, "a plain reply asks no confirmation");
                    replies.push((root, draft.text, now.0));
                }
                MurmureCommand::SendMessage { .. } => panic!("a message in the channel"),
                _ => {}
            }
        }
        replies
    }

    /// The thread's region shows a composer of its own. Enter there sends
    /// the draft as a reply to the root, at the clock's time, and posts
    /// nothing in the channel (`MURMURE-523`).
    #[test]
    fn enter_in_a_thread_sends_a_reply_to_its_root() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, root, _) = threaded(channel);
        assert!(
            page.view.composer().band() > 0.0,
            "the thread has a composer"
        );
        assert!(press_composer(&mut page));
        type_slowly(&mut page, "ok");
        assert!(key(&mut page, "Enter", false, ambre_bus::PageTime(1_234.0)));
        frame_with_fonts(&mut page.view);
        assert_eq!(replies(&page), [(root, "ok".to_owned(), 1_234)]);
        let of = DraftOf::Thread(channel, root);
        assert_eq!(page.view.composer().draft(of), None, "the draft is gone");
    }

    /// Each thread keeps a draft of its own, apart from its channel's. A
    /// reply sent leaves the channel's draft and another thread's
    /// (`MURMURE-524`, `MURMURE-525`).
    #[test]
    fn a_thread_keeps_its_own_draft_apart_from_the_channels() {
        let channel = ChannelId::new();
        let (mut page, mut channel_view, root, own) = threaded(channel);
        std::mem::swap(&mut page.view, &mut channel_view);
        press_composer(&mut page);
        type_slowly(&mut page, "in channel");
        std::mem::swap(&mut page.view, &mut channel_view);
        press_composer(&mut page);
        type_slowly(&mut page, "in thread");
        let of = DraftOf::Thread(channel, root);
        assert_eq!(painted(&mut page, of).as_deref(), Some("in thread"));
        assert_eq!(
            page.view.composer().draft(channel),
            None,
            "not the channel's"
        );

        // Another thread of the channel opens on a draft of its own.
        let pane = page.pane.get();
        let MurmurePane::Thread { root: at, .. } = pane else {
            panic!("a thread is shown");
        };
        let other = MessageRef { message: own, ..at };
        page.pane.set(MurmurePane::Thread {
            channel,
            root: other,
        });
        let elsewhere = DraftOf::Thread(channel, own);
        assert_eq!(painted(&mut page, elsewhere), None, "an empty draft");
        type_slowly(&mut page, "aside");
        page.pane.set(pane);
        assert_eq!(painted(&mut page, of).as_deref(), Some("in thread"));

        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(replies(&page).len(), 1);
        let kept = page.view.composer().draft(elsewhere);
        assert_eq!(kept.as_deref(), Some("aside"), "the other thread's stays");
        frame_with_fonts(&mut channel_view);
        let draft = channel_view.composer().draft(channel);
        assert_eq!(draft.as_deref(), Some("in channel"), "the channel's stays");
    }

    /// The edit action on a reply of the member loads it into the thread's
    /// composer, never the channel's. Enter sends an edit of the reply in
    /// its thread, and no new message (`MURMURE-526`).
    #[test]
    fn the_edit_of_a_reply_loads_the_threads_composer() {
        let channel = ChannelId::new();
        let (mut page, mut channel_view, root, own) = threaded(channel);
        *page.editing.borrow_mut() = Some(PendingEdit {
            channel,
            at: MessageAt::Thread(root),
            message: own,
            text: "mine".to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
        });
        frame_with_fonts(&mut channel_view);
        assert!(
            !channel_view.composer().editing(channel),
            "the channel leaves it"
        );
        assert!(page.editing.borrow().is_some(), "still asked");
        let of = DraftOf::Thread(channel, root);
        assert_eq!(painted(&mut page, of).as_deref(), Some("mine"));
        assert!(page.view.composer().editing(of));
        assert!(page.view.composer().focused(), "the thread takes the keys");

        type_text(&mut page, "!");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let (sent_edits, messages) = edits(&page);
        let [(at, message, draft)] = &sent_edits[..] else {
            panic!("one edit, got {sent_edits:?}");
        };
        assert_eq!((*at, *message), (MessageAt::Thread(root), own));
        assert_eq!((draft.text.as_str(), messages), ("mine!", 0));
    }

    /// Up in the thread's empty composer opens the member's latest reply for
    /// edit, past another member's. The channel's composer stays as it was
    /// (`MURMURE-527`).
    #[test]
    fn up_in_a_threads_empty_composer_edits_the_last_own_reply() {
        let channel = ChannelId::new();
        let (mut page, channel_view, root, _) = threaded(channel);
        press_composer(&mut page);
        frame_with_fonts(&mut page.view);
        assert!(key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0)));
        let of = DraftOf::Thread(channel, root);
        assert_eq!(painted(&mut page, of).as_deref(), Some("mine"));
        assert!(page.view.composer().editing(of));
        assert!(!channel_view.composer().editing(channel));
    }

    /// Escape in the thread's list shows its channel at the root, as the
    /// back control does. In the composer it gives the keys back first
    /// (`MURMURE-528`).
    #[test]
    fn escape_in_a_threads_list_closes_the_thread() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, root, _) = threaded(channel);
        press_composer(&mut page);
        assert!(key(&mut page, "Escape", false, ambre_bus::PageTime(0.0)));
        assert!(!page.view.composer().focused());
        assert!(matches!(page.pane.get(), MurmurePane::Thread { .. }));
        assert!(key(&mut page, "Escape", false, ambre_bus::PageTime(0.0)));
        let MurmurePane::Channel { channel: shown, at } = page.pane.get() else {
            panic!("the channel shows, got {:?}", page.pane.get());
        };
        assert_eq!(shown, channel);
        assert_eq!(at.map(|at| at.message), Some(root), "at the root");
        frame_with_fonts(&mut page.view);
        assert!(
            !key(&mut page, "Escape", false, ambre_bus::PageTime(0.0)),
            "no thread left to close"
        );
    }

    /// Closing a thread with Escape marks neither the channel nor the
    /// thread read (`MURMURE-529`).
    #[test]
    fn closing_a_thread_with_escape_marks_nothing_read() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, _, _) = threaded(channel);
        reads(&page);
        assert!(key(&mut page, "Escape", false, ambre_bus::PageTime(0.0)));
        frame_with_fonts(&mut page.view);
        assert_eq!(reads(&page), (0, 0), "the Escape reads nothing");
    }

    /// A thread's draft outlives its region: the panel closed and opened
    /// again shows it, unfocused (`MURMURE-530`).
    #[test]
    fn a_threads_draft_survives_the_thread_closing() {
        let channel = ChannelId::new();
        let (mut page, channel_view, root, _) = threaded(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "later");
        let pane = page.pane.get();
        key(&mut page, "Escape", false, ambre_bus::PageTime(0.0));
        key(&mut page, "Escape", false, ambre_bus::PageTime(0.0));
        drop(std::mem::replace(&mut page.view, channel_view));
        page.view = page.view.region(300.0, 400.0);
        page.pane.set(pane);
        let of = DraftOf::Thread(channel, root);
        assert_eq!(painted(&mut page, of).as_deref(), Some("later"));
        assert!(!page.view.composer().focused(), "the keys stay put");
    }

    /// Whether each reply sent since the last call asked for a copy in the
    /// channel.
    fn copies(page: &Page) -> Vec<bool> {
        let mut copies = Vec::new();
        while let Ok(command) = page.commands.try_recv() {
            if let LogicCommand::Channel {
                command:
                    MurmureCommand::Reply {
                        also_in_channel, ..
                    },
                ..
            } = command
            {
                copies.push(also_in_channel == ReplyEcho::AlsoInChannel);
            }
        }
        copies
    }

    /// A press on the box under the thread's field checks it and leaves the
    /// keys in the field. Enter then sends the reply to the channel too, and
    /// the box unchecks (`MURMURE-531`). The channel's band has no box.
    #[test]
    fn the_box_sends_the_reply_to_the_channel_too_once() {
        let channel = ChannelId::new();
        let (mut page, channel_view, root, _) = threaded(channel);
        let of = DraftOf::Thread(channel, root);
        let bottom = page.view.size().1 - ALSO_PX / 2.0;
        assert!(press_composer(&mut page));
        assert!(press_at(&mut page, bottom), "the press on the box is taken");
        assert!(page.view.composer().also_in_channel(of) == ReplyEcho::AlsoInChannel);
        assert!(page.view.composer().focused(), "the field keeps the keys");
        type_slowly(&mut page, "ok");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(copies(&page), [true]);
        assert!(
            page.view.composer().also_in_channel(of) == ReplyEcho::ThreadOnly,
            "unchecked"
        );
        type_slowly(&mut page, "again");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(copies(&page), [false], "the next reply stays");
        assert!(!channel_view.composer().on_also(bottom), "no box there");
    }

    /// A tap on the box checks it too, and a second one unchecks it
    /// (`MURMURE-531`).
    #[test]
    fn a_tap_on_the_box_checks_it() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, root, _) = threaded(channel);
        let of = DraftOf::Thread(channel, root);
        let bottom = page.view.size().1 - ALSO_PX / 2.0;
        for checked in [true, false] {
            page.view.touch_down(10.0, bottom, ambre_bus::PageTime(0.0));
            assert_eq!(
                page.view.touch_up(ambre_bus::PageTime(20.0)),
                None,
                "the tap is taken"
            );
            assert_eq!(
                page.view.composer().also_in_channel(of) == ReplyEcho::AlsoInChannel,
                checked
            );
        }
    }

    fn alt_enter(page: &mut Page) -> bool {
        let ev = InputEvent::Key {
            key: "Enter".to_owned(),
            ctrl: false,
            alt: true,
            shift: false,
            meta: false,
        };
        page.view.handle_input(&ev)
    }

    /// Alt+Enter in a thread's composer checks the box and sends nothing.
    /// A second one unchecks it (`MURMURE-534`).
    #[test]
    fn alt_enter_turns_the_box() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, root, _) = threaded(channel);
        let of = DraftOf::Thread(channel, root);
        assert!(press_composer(&mut page));
        type_slowly(&mut page, "ok");
        assert!(alt_enter(&mut page));
        frame_with_fonts(&mut page.view);
        assert!(page.view.composer().also_in_channel(of) == ReplyEcho::AlsoInChannel);
        assert!(copies(&page).is_empty(), "nothing sent");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(copies(&page), [true]);
        assert!(alt_enter(&mut page));
        assert!(alt_enter(&mut page));
        assert!(
            page.view.composer().also_in_channel(of) == ReplyEcho::ThreadOnly,
            "unchecked"
        );
    }

    /// Every run of glyphs the view paints, as glyph counts.
    fn glyph_runs(page: &mut Page) -> Vec<usize> {
        page.view.update();
        page.view.plan_repaint(ambre_bus::PageTime(0.0), false);
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let mut surface = RecordingSurface::new();
        page.view
            .paint_content(&mut surface, &mut font_cx, &mut cache, None);
        surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                ambre_draw::testing::DrawOp::Glyphs { glyph_count, .. } => Some(*glyph_count),
                _ => None,
            })
            .collect()
    }

    /// The box under a thread's composer names Alt+Enter at the desktop
    /// width only, and drops it again below (`MURMURE-544`).
    #[test]
    fn the_box_names_its_chord_at_the_desktop_width_only() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, _, _) = threaded(channel);
        assert!(press_composer(&mut page));
        let narrow = glyph_runs(&mut page);
        page.view.set_desktop(true);
        let mut wide = glyph_runs(&mut page);
        let at = wide
            .iter()
            .zip(narrow.iter().chain(std::iter::repeat(&0)))
            .position(|(w, n)| w != n)
            .expect("one more run at the desktop width");
        assert_eq!(wide.remove(at), crate::composer::ALSO_CHORD.chars().count());
        assert_eq!(wide, narrow, "only the chord is added");
        page.view.set_desktop(false);
        assert_eq!(glyph_runs(&mut page), narrow);
    }

    /// Alt+Enter in a channel's composer sends, as Enter does, since a
    /// channel's draft has no box (`MURMURE-534`).
    #[test]
    fn alt_enter_in_a_channel_sends() {
        let mut page = composing(ChannelId::new());
        assert!(press_composer(&mut page));
        type_text(&mut page, "hi");
        assert!(alt_enter(&mut page));
        frame_with_fonts(&mut page.view);
        assert_eq!(sent(&page).len(), 1);
    }

    /// Each turn of the box by a key is said with its state, since the box
    /// is drawn on the canvas (`MURMURE-535`).
    #[test]
    fn a_turn_of_the_box_by_a_key_is_said() {
        let channel = ChannelId::new();
        let (mut page, _channel_view, _root, _) = threaded(channel);
        assert!(press_composer(&mut page));
        page.said.borrow_mut().clear();
        alt_enter(&mut page);
        alt_enter(&mut page);
        assert_eq!(
            *page.said.borrow(),
            [
                also_said(ReplyEcho::AlsoInChannel),
                also_said(ReplyEcho::ThreadOnly)
            ]
        );
    }

    /// The reply action on a reply also sent to the channel opens its
    /// thread at the root the list holds (`MURMURE-533`).
    #[test]
    fn the_reply_on_a_copy_opens_its_thread() {
        let mut page = page();
        let channel = ChannelId::new();
        let ada = UserId::new();
        let thread = DocumentId::new();
        let root = MessageView {
            thread: Some(ThreadHead {
                document: thread,
                replies: 1,
                last_replier: Some(ada),
            }),
            ..message(ada, ambre_bus::Timestamp(0), "root")
        };
        let copy = MessageView {
            also_in_thread: Some(thread),
            ..message(ada, ambre_bus::Timestamp(1), "ok")
        };
        let segment = SegmentId::new();
        let messages = vec![root.clone(), copy];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        page.pane.set(MurmurePane::channel(channel));
        frame_with_fonts(&mut page.view);
        let _ = reads(&page);

        let y = middle_of(&page.view, 1);
        button(&mut page, PointerButton::Secondary, y);
        let acted = page.acted.borrow_mut().pop().flatten();
        let target = acted.expect("the copy is the target");
        let at = MessageRef {
            segment,
            message: root.id,
        };
        assert_eq!(target.from_thread, Some(FromThread::Root(at)));
        let Some(MessageAction::Open(pane)) = message_action(REPLY, &target, None) else {
            panic!("the reply opens a thread");
        };
        assert_eq!(pane, MurmurePane::Thread { channel, root: at });
    }

    /// A composer on `channel`, whose members are Ada (the writer), Bob and
    /// Carol, with the keys in its field.
    fn mentioning(channel: ChannelId) -> (Page, UserId, UserId) {
        let mut page = composing(channel);
        let (ada, bob, carol) = (UserId::new(), UserId::new(), UserId::new());
        page.me.set(Some(ada));
        *page.members.borrow_mut() = vec![
            (carol, ambre_core::DisplayName::new("Carol").unwrap()),
            (ada, ambre_core::DisplayName::new("Ada").unwrap()),
            (bob, ambre_core::DisplayName::new("Bob").unwrap()),
        ];
        assert!(press_composer(&mut page));
        (page, bob, carol)
    }

    /// The entries of the `@` list the page holds now, `None` when closed.
    fn listed(page: &Page) -> Option<Vec<String>> {
        let lists = page.lists.borrow();
        lists
            .last()
            .and_then(|(_, list)| list.as_ref().map(|l| l.entries.as_slice().to_vec()))
    }

    /// The entry the `@` list chose, as the page holds it.
    fn chosen(page: &Page) -> Option<usize> {
        let lists = page.lists.borrow();
        lists
            .last()
            .and_then(|(_, list)| list.as_ref().map(|l| l.chosen.to_usize()))
    }

    /// The drafts sent since the last call.
    fn drafts_sent(page: &Page) -> Vec<ambre_bus::MessageDraft> {
        let mut drafts = Vec::new();
        while let Ok(command) = page.commands.try_recv() {
            if let LogicCommand::Channel {
                command: MurmureCommand::SendMessage { draft, .. },
                ..
            } = command
            {
                drafts.push(draft);
            }
        }
        drafts
    }

    /// An `@` that starts a word opens the members of the channel, with
    /// `@channel` first and never the writer. What follows filters them, and
    /// a space closes the list (`MURMURE-551`).
    #[test]
    fn an_at_opens_the_members_with_the_channel_first() {
        let channel = ChannelId::new();
        let (mut page, ..) = mentioning(channel);
        type_text(&mut page, "hi @");
        frame_with_fonts(&mut page.view);
        let all = ["@channel", "@Bob", "@Carol"].map(str::to_owned);
        assert_eq!(listed(&page), Some(all.to_vec()));
        let (in_thread, list) = page.lists.borrow().last().cloned().expect("a list");
        assert_eq!(in_thread, Region::Channel, "the channel's region hangs it");
        assert!(
            list.expect("open").at.x > ViewPx::ZERO,
            "it hangs from the caret"
        );
        type_text(&mut page, "c");
        frame_with_fonts(&mut page.view);
        let kept = ["@channel", "@Carol"].map(str::to_owned);
        assert_eq!(listed(&page), Some(kept.to_vec()));
        type_text(&mut page, " ");
        frame_with_fonts(&mut page.view);
        assert_eq!(listed(&page), None, "a space ends the query");
        type_text(&mut page, "mail@x");
        frame_with_fonts(&mut page.view);
        assert_eq!(listed(&page), None, "an @ inside a word opens nothing");
    }

    /// A pick writes the name over the query, then a space, and the message
    /// carries a mention over the name. Tab picks as Enter does, and sends
    /// nothing (`MURMURE-552`).
    #[test]
    fn a_pick_writes_the_name_and_sends_its_mention() {
        let channel = ChannelId::new();
        let (mut page, bob, carol) = mentioning(channel);
        type_text(&mut page, "@b");
        frame_with_fonts(&mut page.view);
        assert!(key(&mut page, "Enter", false, ambre_bus::PageTime(0.0)));
        frame_with_fonts(&mut page.view);
        assert!(drafts_sent(&page).is_empty(), "the pick sent nothing");
        assert_eq!(
            page.view.composer().draft(channel).as_deref(),
            Some("@Bob ")
        );
        type_text(&mut page, "and @car");
        frame_with_fonts(&mut page.view);
        assert!(key(&mut page, "Tab", false, ambre_bus::PageTime(0.0)));
        frame_with_fonts(&mut page.view);
        type_text(&mut page, "hi");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let sent = drafts_sent(&page);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].text, "@Bob and @Carol hi");
        let mentions: Vec<_> = sent[0]
            .mentions
            .iter()
            .map(|m| (m.target, m.range.to_usize_range()))
            .collect();
        let (bob, carol) = (MentionTarget::User(bob), MentionTarget::User(carol));
        assert_eq!(mentions, vec![(bob, 0..4), (carol, 9..15)]);
    }

    /// Text typed before a mention moves it, and a key that cuts into the
    /// name drops it. The message never names a member over other text
    /// (`MURMURE-553`).
    #[test]
    fn an_edit_of_the_draft_keeps_the_mentions_it_leaves_whole() {
        let channel = ChannelId::new();
        let (mut page, bob, _) = mentioning(channel);
        type_text(&mut page, "@bo");
        frame_with_fonts(&mut page.view);
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        key(&mut page, "Home", false, ambre_bus::PageTime(0.0));
        type_text(&mut page, "é ");
        frame_with_fonts(&mut page.view);
        key(&mut page, "End", false, ambre_bus::PageTime(0.0));
        type_text(&mut page, "@channel");
        frame_with_fonts(&mut page.view);
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.composer().draft(channel).as_deref(),
            Some("é @Bob @channel ")
        );
        // Two keys back cut the space and a char of `@channel`.
        key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let sent = drafts_sent(&page);
        assert_eq!(sent.len(), 1, "a broken query opened no list");
        assert_eq!(sent[0].text, "é @Bob @channe");
        let mentions: Vec<_> = sent[0]
            .mentions
            .iter()
            .map(|m| (m.target, m.range.to_usize_range()))
            .collect();
        assert_eq!(mentions, vec![(MentionTarget::User(bob), 2..6)]);
    }

    fn styled(style: ambre_bus::TextStyle, range: std::ops::Range<usize>) -> ambre_bus::StyleMark {
        ambre_bus::StyleMark {
            style,
            range: CharSpan::from_usize_range(range),
        }
    }

    /// A marker becomes its mark the moment it closes, and its chars leave
    /// the draft. One undo gives them back, and the message sent carries the
    /// mark (`MURMURE-197`).
    #[test]
    fn a_closed_marker_becomes_its_mark_and_an_undo_gives_it_back() {
        use ambre_bus::TextStyle::{Bold, Italic};
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "say **done**");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("say done"));
        assert_eq!(page.view.composer().styles(channel), [styled(Bold, 4..8)]);

        chord(&mut page, "z", false);
        assert_eq!(painted(&mut page, channel).as_deref(), Some("say **done**"));
        assert_eq!(page.view.composer().styles(channel), []);
        key(&mut page, "End", false, ambre_bus::PageTime(0.0));
        type_slowly(&mut page, " _now_");
        assert_eq!(
            painted(&mut page, channel).as_deref(),
            Some("say **done** now"),
            "the next marker closes alone, and the undone one stays text"
        );
        type_slowly(&mut page, "!");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let sent = drafts_sent(&page);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].text, "say **done** now!");
        assert_eq!(
            sent[0].styles,
            [styled(Italic, 13..16)],
            "text typed past a mark stays out of it"
        );

        // A marker closed in the frame of Enter becomes its mark first.
        type_text(&mut page, "**go*");
        type_text(&mut page, "*");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let sent = drafts_sent(&page);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            (sent[0].text.as_str(), sent[0].styles.as_slice()),
            ("go", [styled(Bold, 0..2)].as_slice())
        );
    }

    /// The field draws each mark as it is typed. Bold is heavier, code is
    /// mono on a tint, and a link is underlined in the link ink. A range
    /// counts bytes, past a wide char (`MURMURE-564`).
    #[test]
    fn the_field_draws_each_mark_as_it_is_typed() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "é **b** `c` [d](https://ambre.dev)");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("é b c d"));
        let palette = page.view.theme.palette();
        let spans: Vec<_> = page
            .view
            .composer()
            .spans(channel)
            .into_iter()
            .map(|span| {
                let look = (span.weight, span.italic, span.family, span.color);
                (
                    span.range.to_usize_range(),
                    look,
                    span.underline,
                    span.background,
                )
            })
            .collect();
        let code = Some("Monospace".to_owned());
        assert_eq!(
            spans,
            [
                (3..4, (Some(700), false, None, None), false, None),
                (
                    5..6,
                    (None, false, code, None),
                    false,
                    Some(palette.header_hover)
                ),
                (
                    7..8,
                    (None, false, None, Some(palette.selection)),
                    true,
                    None
                ),
            ]
        );
        type_slowly(&mut page, " _e_");
        let spans = page.view.composer().spans(channel);
        assert_eq!(spans.len(), 4);
        assert!(spans[3].italic && spans[3].range == ByteSpan::from_usize_range(9..10));
        // A draft with no mark draws none.
        for _ in 0.."é b c d e".chars().count() {
            key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        }
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().spans(channel), []);
    }

    /// A marker typed inside inline code stays text (`MURMURE-198`).
    #[test]
    fn a_marker_typed_in_code_stays_text() {
        use ambre_bus::TextStyle::Code;
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "`a b`");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("a b"));
        assert_eq!(page.view.composer().styles(channel), [styled(Code, 0..3)]);
        key(&mut page, "ArrowLeft", false, ambre_bus::PageTime(0.0));
        type_slowly(&mut page, "**x**");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("a **x**b"));
        assert_eq!(
            page.view.composer().styles(channel),
            [styled(Code, 0..8)],
            "the marker is text in the code"
        );
    }

    /// The marks the field draws in the indent of a block, in the muted ink:
    /// a quote's rule and a list's dots.
    fn margin_marks(page: &mut Page) -> usize {
        page.view.update();
        page.view.plan_repaint(ambre_bus::PageTime(0.0), false);
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let mut surface = RecordingSurface::new();
        page.view
            .paint_content(&mut surface, &mut font_cx, &mut cache, None);
        let ink = crate::look::margin_ink(page.view.theme.palette());
        let top = f64::from(400.0 - page.view.composer().band());
        let indent = f64::from(SPACE_8) + f64::from(crate::look::BLOCK_INDENT);
        surface.count(|op| {
            matches!(op, ambre_draw::testing::DrawOp::FillRect { rect, color }
                if *color == ink && rect.y0 >= top
                    && rect.x0 >= f64::from(SPACE_8) && rect.x1 <= indent)
        })
    }

    /// A block marker at the start of a line becomes its mark as it closes
    /// (`MURMURE-563`). A quote and a list item close on their line break or
    /// on the send, a code block on its closing fence. A marker under an
    /// open fence stays text. The field draws a rule and a dot in its margin, and
    /// code on its tint (`MURMURE-564`).
    #[test]
    fn a_block_marker_becomes_its_mark_and_the_field_draws_it() {
        use ambre_bus::TextStyle::{CodeBlock, List, Quote};
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "> said");
        shift_key(&mut page, "Enter");
        type_slowly(&mut page, "- one");
        shift_key(&mut page, "Enter");
        type_slowly(&mut page, "- two");
        assert_eq!(
            painted(&mut page, channel).as_deref(),
            Some("said\none\n- two")
        );
        assert_eq!(
            page.view.composer().styles(channel),
            [styled(Quote, 0..5), styled(List, 5..9)]
        );
        assert_eq!(margin_marks(&mut page), 2, "a rule and one dot");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let sent = drafts_sent(&page);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].text, "said\none\ntwo");
        assert_eq!(
            sent[0].styles,
            [styled(Quote, 0..5), styled(List, 5..12)],
            "the send closes the last item, and the items read as one list"
        );

        type_slowly(&mut page, "```");
        shift_key(&mut page, "Enter");
        type_slowly(&mut page, "> x");
        shift_key(&mut page, "Enter");
        assert_eq!(
            page.view.composer().styles(channel),
            [],
            "under an open fence"
        );
        type_slowly(&mut page, "```");
        assert_eq!(painted(&mut page, channel).as_deref(), Some("> x"));
        assert_eq!(
            page.view.composer().styles(channel),
            [styled(CodeBlock, 0..3)]
        );
        let palette = page.view.theme.palette();
        let spans = page.view.composer().spans(channel);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].family.as_deref(), Some("Monospace"));
        assert_eq!(spans[0].background, Some(palette.header_hover));
        assert_eq!(margin_marks(&mut page), 0, "code draws no margin mark");
    }

    /// A quote in the field draws each of its lines past its rule, a wrapped
    /// one too. The line after it starts at the field's edge again
    /// (`MURMURE-570`).
    #[test]
    fn the_field_draws_a_blocks_lines_past_its_margin_mark() {
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        let long = "word ".repeat(20);
        type_slowly(&mut page, &format!("> {long}"));
        shift_key(&mut page, "Enter");
        type_slowly(&mut page, "plain");
        page.view.update();
        page.view.plan_repaint(ambre_bus::PageTime(0.0), false);
        let (mut font_cx, mut cache) = (crate::paint::tests::fonts(), TextCache::new(1.0));
        let mut surface = RecordingSurface::new();
        page.view
            .paint_content(&mut surface, &mut font_cx, &mut cache, None);
        let ink = page.view.theme.palette().cell_text;
        let top = f64::from(400.0 - page.view.composer().band());
        let mut lines: Vec<(f64, f64)> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                ambre_draw::testing::DrawOp::Glyphs {
                    color: Some(c),
                    transform,
                    ..
                } if *c == ink && transform.translation().y >= top => {
                    let at = transform.translation();
                    Some((at.y, at.x))
                }
                _ => None,
            })
            .collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        let [block @ .., (_, after)] = lines.as_slice() else {
            panic!("the draft's lines, got {lines:?}");
        };
        let edge = f64::from(SPACE_8);
        let indent = edge + f64::from(crate::look::BLOCK_INDENT);
        assert!(block.len() >= 2, "the quote wraps, got {block:?}");
        assert!(
            block.iter().all(|(_, x)| (x - indent).abs() < 0.5),
            "each line of the quote starts past its rule, got {block:?}"
        );
        assert!((after - edge).abs() < 0.5, "the next line at the edge");
    }

    /// `Ctrl+B` puts bold on the selection and takes it off again. With no
    /// selection the chord changes nothing (`MURMURE-431`). The style is a
    /// mark over the draft of the inline editor, sent with it (`MURMURE-193`).
    #[test]
    fn a_chord_puts_its_style_on_the_selection() {
        use ambre_bus::TextStyle::{Bold, Italic};
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "one two");
        assert!(
            chord(&mut page, "b", false),
            "no selection: the chord is taken"
        );
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().styles(channel), []);
        for _ in 0..3 {
            shift_key(&mut page, "ArrowLeft");
        }
        frame_with_fonts(&mut page.view);
        chord(&mut page, "b", false);
        chord(&mut page, "I", false);
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.composer().styles(channel),
            [styled(Bold, 4..7), styled(Italic, 4..7)]
        );
        chord(&mut page, "b", false);
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().styles(channel), [styled(Italic, 4..7)]);
        assert!(!chord(&mut page, "q", false), "no style takes `q`");
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let sent = drafts_sent(&page);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].styles, [styled(Italic, 4..7)]);
    }

    /// A selection in the composer shows the format bar, with the Murmure
    /// marks alone, and the bar leaves with the selection. A press toggles
    /// its style as the chord does (`MURMURE-432`).
    #[test]
    fn a_selection_shows_the_format_bar_and_a_press_toggles_its_style() {
        use crate::markers::Cover;
        use ambre_bus::TextStyle::{Bold, Code, Italic};
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        type_slowly(&mut page, "one two");
        let last = |page: &Page| page.bars.borrow().last().cloned();
        assert_eq!(last(&page), None, "no selection, no bar");
        for _ in 0..3 {
            shift_key(&mut page, "ArrowLeft");
        }
        frame_with_fonts(&mut page.view);
        let (in_thread, bar) = last(&page).expect("a bar was handed over");
        let bar = bar.expect("the selection shows it");
        assert_eq!(in_thread, Region::Channel);
        let none = [
            (Bold, Cover::None),
            (Italic, Cover::None),
            (Code, Cover::None),
        ];
        assert_eq!(bar.controls, none);
        *page.press.borrow_mut() = Some((Region::Thread, Bold));
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.composer().styles(channel),
            [],
            "the thread's press stays"
        );
        *page.press.borrow_mut() = Some((Region::Channel, Bold));
        frame_with_fonts(&mut page.view);
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().styles(channel), [styled(Bold, 4..7)]);
        let (_, bar) = last(&page).expect("the bar follows the press");
        let whole = [
            (Bold, Cover::Whole),
            (Italic, Cover::None),
            (Code, Cover::None),
        ];
        assert_eq!(bar.map(|b| b.controls), Some(whole.to_vec()));
        key(&mut page, "ArrowRight", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(
            last(&page),
            Some((Region::Channel, None)),
            "the bar leaves with the selection"
        );
        let shown = page.bars.borrow().len();
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.bars.borrow().len(),
            shown,
            "a bar unchanged is not handed again"
        );
    }

    /// The draft is said to a screen reader as the keys land on it. The
    /// focus, a move, a delete, a selection and an undo say it, and a typed
    /// key does not (`MURMURE-571` to `MURMURE-575`).
    #[test]
    fn the_keys_on_the_draft_are_said_once_they_land() {
        let mut page = composing(ChannelId::new());
        let last = |page: &Page| page.said.borrow().last().cloned();
        press_composer(&mut page);
        frame_with_fonts(&mut page.view);
        assert_eq!(last(&page).as_deref(), Some("blank"), "the focus says it");
        type_slowly(&mut page, "one two");
        assert_eq!(last(&page).as_deref(), Some("blank"), "a typed key is not");
        key(&mut page, "ArrowLeft", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(last(&page).as_deref(), Some("o"));
        chord(&mut page, "ArrowLeft", false);
        frame_with_fonts(&mut page.view);
        assert_eq!(last(&page).as_deref(), Some("two"));
        key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(last(&page).as_deref(), Some("space"));
        shift_key(&mut page, "ArrowLeft");
        frame_with_fonts(&mut page.view);
        assert_eq!(last(&page).as_deref(), Some("e, selected"));
        chord(&mut page, "z", false);
        frame_with_fonts(&mut page.view);
        assert_eq!(last(&page).as_deref(), Some("one two"));
        let count = page.said.borrow().len();
        frame_with_fonts(&mut page.view);
        assert_eq!(page.said.borrow().len(), count, "a key is said once");
    }

    /// A line move on a draft wider than the field says the line it draws.
    /// That line is one wrapped part of a draft with no break (`MURMURE-576`).
    #[test]
    fn a_line_move_on_a_wrapped_draft_says_the_line_drawn() {
        let mut page = composing(ChannelId::new());
        let last = |page: &Page| page.said.borrow().last().cloned();
        press_composer(&mut page);
        // Shorter than an announcement holds, so a clip cannot pass for a line.
        let draft = ["word"; 30].join(" ");
        page.view.paste(&draft);
        frame_with_fonts(&mut page.view);
        key(&mut page, "Home", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let line = last(&page).expect("the line said");
        assert!(line.starts_with("word"), "a line of the draft: {line}");
        assert!(
            line.len() < draft.len(),
            "the whole draft, not the line drawn"
        );
    }

    /// The rows a page's shelf keeps in the store, by key.
    type Rows =
        Rc<RefCell<std::collections::BTreeMap<ambre_bus::DraftKey, crate::drafts::DraftRow>>>;

    /// Give the page's shelf `me` and a store of `rows`, as the launcher does
    /// once the session and the store are ready.
    fn keep_drafts(page: &Page, me: UserId, rows: &Rows) {
        use crate::drafts::DraftWrite;
        let store = Rc::clone(rows);
        page.view.reads.drafts.set_sink(Rc::new(move |write| {
            let mut store = store.borrow_mut();
            match write {
                DraftWrite::Put { key, row } => {
                    store.insert(key, row);
                }
                DraftWrite::Delete { key } => {
                    store.remove(&key);
                }
                DraftWrite::DeleteRange { lower, upper } => {
                    store.retain(|key, _| !(&lower..=&upper).contains(&key));
                }
                DraftWrite::DeleteAll => store.clear(),
            }
        }));
        let kept = rows.borrow().clone().into_iter().collect();
        page.view.reads.drafts.load(me, kept);
    }

    /// A draft typed and not sent is in the store. A page built anew shows it
    /// in its composer once the rows are read, with no key pressed.
    /// No command carried it meanwhile (`MURMURE-210`, `MURMURE-211`).
    #[test]
    fn a_draft_comes_back_after_a_reload() {
        let (channel, me) = (ChannelId::new(), UserId::new());
        let rows = Rows::default();
        let mut page = composing(channel);
        keep_drafts(&page, me, &rows);
        press_composer(&mut page);
        page.view.paste("not sent yet");
        frame_with_fonts(&mut page.view);
        assert_eq!(rows.borrow().len(), 1, "the draft is in the store");
        while let Ok(command) = page.commands.try_recv() {
            let command = format!("{command:?}");
            assert!(
                !command.contains("not sent yet"),
                "the draft left: {command}"
            );
        }

        let mut again = composing(channel);
        assert_eq!(again.view.composer().draft(channel), None);
        keep_drafts(&again, me, &rows);
        frame_with_fonts(&mut again.view);
        assert_eq!(
            again.view.composer().draft(channel).as_deref(),
            Some("not sent yet")
        );
        assert!(
            !again.view.composer().focused(),
            "the keys stay where they are"
        );
    }

    /// A message sent takes its row out of the store, and an empty draft
    /// keeps none (`MURMURE-212`).
    #[test]
    fn a_sent_draft_leaves_the_store() {
        let (channel, me) = (ChannelId::new(), UserId::new());
        let rows = Rows::default();
        let mut page = composing(channel);
        keep_drafts(&page, me, &rows);
        press_composer(&mut page);
        page.view.paste("hello");
        frame_with_fonts(&mut page.view);
        assert_eq!(rows.borrow().len(), 1);
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(drafts_sent(&page).len(), 1, "the message left");
        assert!(rows.borrow().is_empty(), "its draft went with it");
    }

    /// A channel lost takes its draft from the store and from the composer.
    /// The composer writes it back no more; another channel keeps its own
    /// (`MURMURE-215`, `MURMURE-217`).
    #[test]
    fn a_lost_channel_takes_its_draft_from_the_composer() {
        let (lost, kept, me) = (ChannelId::new(), ChannelId::new(), UserId::new());
        let rows = Rows::default();
        let mut page = composing(kept);
        keep_drafts(&page, me, &rows);
        press_composer(&mut page);
        page.view.paste("stays");
        frame_with_fonts(&mut page.view);
        list(&page, lost, &["b"]);
        page.pane.set(MurmurePane::Channel {
            channel: lost,
            at: None,
        });
        frame_with_fonts(&mut page.view);
        press_composer(&mut page);
        page.view.paste("goes");
        frame_with_fonts(&mut page.view);
        assert_eq!(rows.borrow().len(), 2);
        page.view.reads.drafts.forget_channel(lost);
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().draft(lost), None);
        let left: Vec<String> = rows
            .borrow()
            .values()
            .cloned()
            .filter_map(|row| String::from_utf8(row.into_bytes()).ok())
            .collect();
        assert_eq!(left.len(), 1, "one draft left: {left:?}");
        assert!(left[0].contains("stays"));
        assert_eq!(page.view.composer().draft(kept).as_deref(), Some("stays"));
    }

    /// An edit loads the styles of its message. One that changes the styles
    /// alone is sent, and one that changes nothing is not (`MURMURE-562`).
    #[test]
    fn an_edit_of_the_styles_alone_is_sent() {
        use ambre_bus::TextStyle::Bold;
        let channel = ChannelId::new();
        let mut page = composing(channel);
        press_composer(&mut page);
        let message = MessageId::new();
        let edit = |page: &Page| {
            *page.editing.borrow_mut() = Some(PendingEdit {
                channel,
                at: MessageAt::Segment(SegmentId::new()),
                message,
                text: "hi you".to_owned(),
                mentions: Vec::new(),
                styles: vec![styled(Bold, 0..2)],
            });
        };
        edit(&page);
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().styles(channel), [styled(Bold, 0..2)]);
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(edits(&page), (Vec::new(), 0), "nothing changed");

        edit(&page);
        frame_with_fonts(&mut page.view);
        chord(&mut page, "a", false);
        chord(&mut page, "b", false);
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        let (sent, messages) = edits(&page);
        assert_eq!(messages, 0);
        let [(_, edited, draft)] = &sent[..] else {
            panic!("one edit, got {sent:?}");
        };
        assert_eq!(*edited, message);
        assert_eq!(draft.text, "hi you");
        assert_eq!(draft.styles, [styled(Bold, 0..6)]);
    }

    /// While the list shows, Up and Down move its choice round it and say
    /// the entry. Escape closes it and keeps the draft and the focus, and the
    /// list stays closed until its query ends (`MURMURE-554`). None of it
    /// changes the draft or sends it (`MURMURE-556`).
    #[test]
    fn the_list_takes_its_keys_and_escape_closes_it_until_the_query_ends() {
        let channel = ChannelId::new();
        let (mut page, ..) = mentioning(channel);
        type_text(&mut page, "@");
        frame_with_fonts(&mut page.view);
        assert_eq!(chosen(&page), Some(0));
        assert!(key(&mut page, "ArrowDown", false, ambre_bus::PageTime(0.0)));
        frame_with_fonts(&mut page.view);
        assert_eq!(chosen(&page), Some(1));
        assert_eq!(page.said.borrow().last().map(String::as_str), Some("@Bob"));
        key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0));
        key(&mut page, "ArrowUp", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(chosen(&page), Some(2), "Up from the first wraps");
        assert!(key(&mut page, "Escape", false, ambre_bus::PageTime(0.0)));
        frame_with_fonts(&mut page.view);
        assert_eq!(listed(&page), None);
        assert!(page.view.composer().focused(), "Escape kept the keys");
        assert_eq!(page.view.composer().draft(channel).as_deref(), Some("@"));
        type_text(&mut page, "b");
        frame_with_fonts(&mut page.view);
        assert_eq!(listed(&page), None, "the query Escape closed stays closed");
        type_text(&mut page, " @");
        frame_with_fonts(&mut page.view);
        assert!(listed(&page).is_some(), "a new query opens it again");
        assert!(drafts_sent(&page).is_empty());
    }

    /// A click the page took on an entry picks it at the next frame, in the
    /// region it names alone (`MURMURE-555`).
    #[test]
    fn a_click_on_an_entry_picks_it_in_its_region() {
        let channel = ChannelId::new();
        let (mut page, ..) = mentioning(channel);
        type_text(&mut page, "@");
        frame_with_fonts(&mut page.view);
        page.picked.set(Some((Region::Thread, OfferIndex::new(1))));
        frame_with_fonts(&mut page.view);
        assert_eq!(page.view.composer().draft(channel).as_deref(), Some("@"));
        assert!(page.picked.get().is_some(), "the thread's pick waits");
        page.picked.set(Some((Region::Channel, OfferIndex::new(1))));
        frame_with_fonts(&mut page.view);
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.composer().draft(channel).as_deref(),
            Some("@Bob ")
        );
        assert_eq!(listed(&page), None, "the pick closed the list");
        assert!(page.view.composer().focused());
    }

    /// A cancelled `@channel` gives the channel's composer its draft back,
    /// mentions and all, so the next Enter sends it whole. A draft typed
    /// since stays (`MURMURE-558`).
    #[test]
    fn a_cancelled_at_channel_gives_its_draft_back() {
        let channel = ChannelId::new();
        let (mut page, bob, _) = mentioning(channel);
        let draft = ambre_bus::MessageDraft {
            text: "@channel @Bob hi".to_owned(),
            styles: Vec::new(),
            mentions: vec![
                Mention {
                    target: MentionTarget::Channel,
                    range: CharSpan::from_usize_range(0..8),
                },
                Mention {
                    target: MentionTarget::User(bob),
                    range: CharSpan::from_usize_range(9..13),
                },
            ],
        };
        let asked = AskedAll {
            channel,
            members: 12,
            held: HeldSend {
                draft: draft.clone(),
                attachments: Vec::new(),
                reply: None,
            },
        };
        // A draft typed since keeps its place.
        type_text(&mut page, "later");
        *page.returned.borrow_mut() = Some(asked.clone());
        frame_with_fonts(&mut page.view);
        assert!(page.returned.borrow().is_none(), "taken once");
        assert_eq!(
            page.view.composer().draft(channel).as_deref(),
            Some("later")
        );
        // An empty composer takes it back.
        for _ in 0.."later".len() {
            key(&mut page, "Backspace", false, ambre_bus::PageTime(0.0));
        }
        frame_with_fonts(&mut page.view);
        // A reply's return waits for the thread's region.
        let reply = AskedAll {
            held: HeldSend {
                reply: Some(HeldReply {
                    root: MessageId::new(),
                    also_in_channel: ReplyEcho::ThreadOnly,
                }),
                ..asked.held.clone()
            },
            ..asked.clone()
        };
        *page.returned.borrow_mut() = Some(reply);
        frame_with_fonts(&mut page.view);
        assert!(
            page.returned.borrow().is_some(),
            "the thread's return waits"
        );
        *page.returned.borrow_mut() = Some(asked);
        frame_with_fonts(&mut page.view);
        assert_eq!(
            page.view.composer().draft(channel).as_deref(),
            Some(draft.text.as_str())
        );
        assert!(page.view.composer().focused());
        key(&mut page, "Enter", false, ambre_bus::PageTime(0.0));
        frame_with_fonts(&mut page.view);
        assert_eq!(drafts_sent(&page), vec![draft]);
    }
}
