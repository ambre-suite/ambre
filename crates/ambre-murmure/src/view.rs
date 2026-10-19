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
use std::rc::Rc;

use ambre_bus::conduit::{Receiver, Sender};
use ambre_bus::{DocumentId, LogicCommand, MurmureCommand, SegmentId, Timestamp, UiEvent, UserId};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache, Theme,
    ThemeControl,
};
use ambre_draw::kinetic::{Kinetic, Release, ScrollStep};
use ambre_draw::{DrawSurface, FontContext, Point, Rect};

use crate::MurmurePane;
use crate::list::{Conversations, MessageList, ReadMarks, RowKey, UnreadLine};
use crate::paint::{Painted, Shown, paint_list};

/// The id of the thread's canvas, in the panel. The shell names it, and the
/// host binds a cell to it while it is in the page.
pub const THREAD_CANVAS: &str = "murmure-thread-canvas";

/// What the page tells a region: the pane, a member's name, a time.
#[derive(Clone)]
pub struct PageReads {
    /// The pane the shell shows now.
    pub pane: Rc<dyn Fn() -> MurmurePane>,
    /// The name of a member, `None` when the page knows none.
    pub name_of: Rc<dyn Fn(UserId) -> Option<String>>,
    /// A time, in the reader's language and zone (`MURMURE-418`).
    pub time_of: Rc<dyn Fn(Timestamp) -> String>,
    /// The page's clock, in milliseconds: the wheel reads it.
    pub now_ms: Rc<dyn Fn() -> f64>,
}

/// The height of one line of a wheel that steps by lines.
const WHEEL_LINE_PX: f32 = 40.0;

/// How far down the viewport a message the pane names comes to rest.
const AT_FRACTION: f32 = 0.25;

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

/// Which list a region shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    /// The channel the pane shows, in the content canvas.
    Channel,
    /// The thread the pane shows, in the panel.
    Thread,
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
    dirty: bool,
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
        view
    }

    /// The size this region lays out in, in logical pixels.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Where the pointer last was over this region.
    pub fn hover(&self) -> Option<Point> {
        self.hover
    }

    /// What the last paint did, `None` before the first.
    pub fn last_painted(&self) -> Option<Painted> {
        self.last
    }

    /// Drain the inbound queue into the lists, so a full queue never stalls
    /// the Logic thread.
    pub fn update(&mut self) {
        let Some(from_logic) = &self.from_logic else {
            return;
        };
        let mut lists = self.lists.borrow_mut();
        while let Ok(ev) = from_logic.try_recv() {
            if lists.conversations.absorb(ev) {
                lists.revision += 1;
            }
        }
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

    /// A finger went down: it stops any motion left.
    pub fn touch_down(&mut self, x: f32, y: f32, now_ms: f64) {
        self.kinetic.press(x, y, now_ms);
    }

    /// A finger moved: past the tap slop the list follows it. Answers
    /// whether the list moved.
    pub fn touch_move(&mut self, x: f32, y: f32, now_ms: f64) -> bool {
        let Some(step) = self.kinetic.sample(x, y, now_ms) else {
            return false;
        };
        self.scroll_by(step);
        true
    }

    /// A finger lifted: a pan released with speed leaves a flick. Answers
    /// the point of a tap.
    pub fn touch_up(&mut self, now_ms: f64) -> Option<(f32, f32)> {
        match self.kinetic.release(now_ms) {
            Release::Tap { x, y } => Some((x, y)),
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

    pub fn plan_repaint(&mut self, now_ms: f64, _hud_tick: bool) -> RepaintPlan {
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
        if let Some(step) = self.kinetic.tick(now_ms)
            && step.y() != 0.0
        {
            self.scroll_by(step);
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
        let painted = paint_list(
            surface,
            font_cx,
            text_cache,
            Shown {
                list,
                unread,
                anchor,
            },
            self.size,
            self.theme.palette(),
            &*self.reads.name_of,
            &*self.reads.time_of,
        );
        self.last = Some(painted);
        self.settle(&painted, &keys);
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

    /// Hand `command` to the Logic thread. A full channel drops it, and the
    /// next paint that calls for it asks again.
    fn send(&self, command: LogicCommand) {
        if let Some(to_logic) = &self.to_logic
            && let Err(e) = to_logic.try_send(command)
        {
            tracing::warn!("Murmure view command dropped, Logic channel full ({e:?})");
        }
    }

    /// Rest the list where the pane shows it: at the message it names, a
    /// quarter down, or on the latest row (`MURMURE-150`).
    fn rest_on(&mut self, pane: MurmurePane) {
        self.rest = match (self.region, pane) {
            (Region::Channel, MurmurePane::Channel { at: Some(at), .. }) => Rest::Row {
                key: RowKey::Message(at.message),
                anchor_y: self.size.1 * AT_FRACTION,
            },
            _ => Rest::Latest,
        };
        self.top_row = None;
        self.asked_older = None;
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

    /// The region follows the pointer, and the wheel scrolls it. A trackpad
    /// glide that pauses coasts on; a notch never does (`KINETIC-002`). The
    /// pinch chord is the page's zoom, so the list leaves it.
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        match *ev {
            InputEvent::PointerDown { x, y, .. }
            | InputEvent::PointerMove { x, y }
            | InputEvent::PointerUp { x, y, .. } => self.hover = Some(Point::new(x, y)),
            InputEvent::Wheel {
                dy,
                delta_mode,
                ctrl_key: false,
                ..
            } => {
                let unit = match delta_mode {
                    1 => WHEEL_LINE_PX,
                    2 => self.size.1,
                    _ => 1.0,
                };
                let now_ms = (self.reads.now_ms)();
                let step = self
                    .kinetic
                    .wheel(0.0, dy as f32 * unit, delta_mode != 0, now_ms);
                self.scroll_by(step);
                return true;
            }
            _ => {}
        }
        false
    }

    /// A Murmure page binds no document, so a route swap changes nothing.
    pub fn set_document(&mut self, _document_id: DocumentId) {}
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use ambre_bus::conduit::local;
    use ambre_bus::{ChannelId, MessageView, MurmureEvent};
    use ambre_draw::testing::RecordingSurface;

    use super::*;
    use crate::MessageRef;
    use crate::list::tests::message;

    struct Page {
        view: MurmureView,
        pane: Rc<Cell<MurmurePane>>,
        clock: Rc<Cell<f64>>,
        events: Sender<UiEvent>,
        commands: Receiver<LogicCommand>,
    }

    fn page() -> Page {
        let (events, from_logic) = local(64);
        let (to_logic, commands) = local(64);
        let pane = Rc::new(Cell::new(MurmurePane::default()));
        let shown = Rc::clone(&pane);
        let clock = Rc::new(Cell::new(0.0));
        let now = Rc::clone(&clock);
        let reads = PageReads {
            pane: Rc::new(move || shown.get()),
            name_of: Rc::new(|_| Some("Ada".to_owned())),
            time_of: Rc::new(|_| "t".to_owned()),
            now_ms: Rc::new(move || now.get()),
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
            view,
            pane,
            clock,
            events,
            commands,
        }
    }

    /// Plan a frame and paint it when the plan asks, as the frame loop does.
    fn frame(view: &mut MurmureView) -> bool {
        frame_at(view, 0.0)
    }

    fn frame_at(view: &mut MurmureView, now_ms: f64) -> bool {
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
            .map(|(i, text)| message(ada, i as u64, text))
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
        let root = message(ada, 0, "root");
        let segment = SegmentId::new();
        let messages = vec![root.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let replies = ["one", "two", "three"].map(|text| message(ada, 1, text));
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
            messages: vec![message(ada, 4, "four")],
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
        let (read, unread) = (message(ada, 0, "a"), message(ada, 1, "b"));
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

    /// A wheel event at `now_ms`, `delta_mode` as the browser names it.
    fn wheel(page: &mut Page, dy: f64, delta_mode: u32, now_ms: f64) -> bool {
        page.clock.set(now_ms);
        let ev = InputEvent::Wheel {
            dx: 0.0,
            dy,
            delta_mode,
            ctrl_key: false,
        };
        page.view.handle_input(&ev)
    }

    /// `count` messages of one author, written from `from` on, in one new
    /// segment of `channel`.
    fn segment(page: &Page, channel: ChannelId, from: u64, count: u64) -> Vec<MessageView> {
        let ada = UserId::new();
        let messages: Vec<_> = (from..from + count)
            .map(|ts| message(ada, ts, "hello"))
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
        assert!(wheel(&mut page, -3.0, 1, 10.0), "the wheel is taken");
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
        wheel(&mut page, -100_000.0, 0, 10.0);
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
        wheel(&mut page, -50.0, 0, 20.0);
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
        wheel(&mut page, -100_000.0, 0, 30.0);
        assert!(frame(&mut page.view));
        assert_eq!(older(&page).len(), 1, "the new oldest segment asks again");
    }

    /// A trackpad glide that pauses keeps the list moving through the
    /// shared model; a notch never does (`MURMURE-417`).
    #[test]
    fn a_paused_glide_coasts_and_a_notch_never_does() {
        let (mut page, _, _) = long_channel(400);
        frame(&mut page.view);
        wheel(&mut page, -2_000.0, 0, 10.0);
        frame_at(&mut page.view, 10.0);
        for (i, now) in [1_000.0, 1_010.0, 1_020.0].into_iter().enumerate() {
            wheel(&mut page, -10.0, 0, now);
            frame_at(&mut page.view, now + i as f64);
        }
        let paused = page.view.resting_on();
        frame_at(&mut page.view, 1_100.0);
        assert!(page.view.scrolling());
        frame_at(&mut page.view, 1_116.0);
        frame_at(&mut page.view, 1_132.0);
        assert_ne!(page.view.resting_on(), paused, "the glide coasts on");

        page.view.kinetic.stop();
        wheel(&mut page, -1.0, 1, 2_000.0);
        frame_at(&mut page.view, 2_000.0);
        let notched = page.view.resting_on();
        assert!(!page.view.scrolling());
        frame_at(&mut page.view, 2_200.0);
        assert_eq!(page.view.resting_on(), notched, "a notch leaves no motion");
    }

    /// A motion that meets the latest row stops there.
    #[test]
    fn a_motion_stops_at_the_latest_row() {
        let (mut page, _, _) = long_channel(400);
        frame(&mut page.view);
        wheel(&mut page, -60.0, 0, 10.0);
        frame_at(&mut page.view, 10.0);
        for now in [1_000.0, 1_010.0, 1_020.0] {
            wheel(&mut page, 10.0, 0, now);
        }
        let mut now = 1_100.0;
        while now < 1_300.0 {
            frame_at(&mut page.view, now);
            now += 16.0;
        }
        assert_eq!(page.view.resting_on(), None);
        assert!(!page.view.scrolling(), "the edge stops the motion");
    }

    /// A finger drags the list, and one that never left the slop is a tap
    /// (`MURMURE-417`).
    #[test]
    fn a_finger_drags_the_list_and_a_still_one_taps() {
        let (mut page, _, _) = long_channel(200);
        frame(&mut page.view);
        page.view.touch_down(10.0, 100.0, 0.0);
        assert!(!page.view.touch_move(10.0, 102.0, 10.0), "inside the slop");
        assert!(page.view.touch_move(10.0, 300.0, 20.0));
        frame_at(&mut page.view, 20.0);
        assert!(
            page.view.resting_on().is_some(),
            "the list followed the finger"
        );
        page.view.touch_cancel();
        assert!(!page.view.scrolling());
        page.view.touch_down(10.0, 100.0, 100.0);
        assert_eq!(page.view.touch_up(150.0), Some((10.0, 100.0)));
        page.view.touch_down(10.0, 100.0, 200.0);
        page.view.touch_move(10.0, 300.0, 300.0);
        assert_eq!(page.view.touch_up(1_000.0), None);
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

        let later = message(UserId::new(), 9_000, "later");
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
}
