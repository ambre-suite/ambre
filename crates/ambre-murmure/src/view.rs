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
use ambre_bus::{
    ChannelId, DocumentId, LogicCommand, MessageAt, MessageId, MurmureCommand, SegmentId,
    Timestamp, UiEvent, UserId,
};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache, Theme,
    ThemeControl,
};
use ambre_draw::kinetic::{Kinetic, Release, ScrollStep};
use ambre_draw::{DrawSurface, FontContext, Point, Rect};

use crate::MurmurePane;
use crate::commands::MessageTarget;
use crate::list::{Conversations, JUMP_HINT, MessageList, ReadMarks, RowKey, UnreadLine};
use crate::paint::{Painted, Shown, paint_jump, paint_list};

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
    /// Say a text to a screen reader, through the polite live region of the
    /// content (`CHROME-023`). It holds no element that takes the focus.
    pub announce: Rc<dyn Fn(&str)>,
    /// Hand the page the message a right-click or a long press fell on,
    /// `None` off any message. The page opens its actions; nothing is sent
    /// (`MURMURE-424`).
    pub act_on: Rc<dyn Fn(Option<MessageTarget>)>,
}

/// The id of the content canvas, which the channel's region paints. The
/// shell names it.
pub const CHANNEL_CANVAS: &str = "ambre-canvas";

/// The button of a pointer event that opens a menu: the right one.
const SECONDARY_BUTTON: i16 = 2;

/// The height of one line of a wheel that steps by lines.
const WHEEL_LINE_PX: f32 = 40.0;

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

/// Which list a region shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    /// The channel the pane shows, in the content canvas.
    Channel,
    /// The thread the pane shows, in the panel.
    Thread,
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
            jump: None,
            focus: None,
            reveal: false,
            heard: (None, None),
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
        self.last.clone()
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

    /// Move the list by one viewport, towards the latest row for a positive
    /// `pages`. The model takes it as a wheel step of a page (`MURMURE-504`).
    fn page(&mut self, pages: f32) {
        let now_ms = (self.reads.now_ms)();
        let step = self.kinetic.wheel(0.0, pages * self.size.1, true, now_ms);
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
        if self.on_jump(at) {
            return None;
        }
        let i = self.last.as_ref()?.row_at(y)?;
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
        Some(MessageTarget {
            channel,
            at,
            message: message.id,
            text: message.text.clone(),
            notice: message.notice.is_some(),
            pressed: pressed.then_some((x, y)),
            canvas: match self.region {
                Region::Channel => CHANNEL_CANVAS,
                Region::Thread => THREAD_CANVAS,
            },
        })
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
        } else if bottom > self.size.1 + REVEAL_SLACK_PX {
            (bottom - self.size.1).min(top)
        } else {
            return;
        };
        let now_ms = (self.reads.now_ms)();
        let step = self.kinetic.wheel(0.0, over, true, now_ms);
        self.scroll_by(step);
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
    /// the point of a tap, unless it fell on the control back to the latest
    /// message, which it presses.
    pub fn touch_up(&mut self, now_ms: f64) -> Option<(f32, f32)> {
        match self.kinetic.release(now_ms) {
            Release::Tap { x, y } if self.on_jump(Point::new(x.into(), y.into())) => {
                self.rest_at_latest();
                None
            }
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
        if let Some((x, y)) = self.kinetic.held(now_ms) {
            // A finger held still: the actions of its message open where it
            // went down (`MURMURE-423`).
            (self.reads.act_on)(self.target_at(x, y, true));
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
        let at_bottom = self.rest == Rest::Latest;
        let focus = self
            .focus
            .and_then(|key| keys.iter().position(|k| *k == key));
        if focus.is_none() {
            self.focus = None;
        }
        let painted = paint_list(
            surface,
            font_cx,
            text_cache,
            Shown {
                list,
                unread,
                anchor,
                focus,
            },
            self.size,
            self.theme.palette(),
            &*self.reads.name_of,
            &*self.reads.time_of,
        );
        self.last = Some(painted.clone());
        self.settle(&painted, &keys);
        if self.reveal {
            self.reveal_focus(&painted, focus, &keys);
        }
        let higher_up = !painted.latest_shown && !list.is_empty();
        let palette = self.theme.palette();
        let shown_before = self.jump.is_some();
        self.jump = higher_up.then(|| paint_jump(surface, font_cx, text_cache, self.size, palette));
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
        self.focus = None;
        self.reveal = false;
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

    /// The region follows the pointer, and the wheel scrolls it. A trackpad
    /// glide that pauses coasts on; a notch never does (`KINETIC-002`). The
    /// pinch chord is the page's zoom, so the list leaves it. Page Up, Page
    /// Down and End jump through the list. Up and Down move the focus, and a
    /// key with a modifier is the browser's (`MURMURE-504`, `MURMURE-505`,
    /// `MURMURE-205`). The ContextMenu key and Shift+F10 open the actions of
    /// the focused message (`MURMURE-511`).
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        match *ev {
            InputEvent::PointerDown { x, y, button, .. } => {
                self.hover = Some(Point::new(x, y));
                if button == SECONDARY_BUTTON {
                    // The page's menu event follows, and opens the actions of
                    // the message found here (`MURMURE-422`).
                    (self.reads.act_on)(self.target_at(x as f32, y as f32, false));
                }
            }
            InputEvent::PointerMove { x, y } => {
                self.hover = Some(Point::new(x, y));
            }
            InputEvent::PointerUp { x, y, .. } => {
                let at = Point::new(x, y);
                self.hover = Some(at);
                if self.on_jump(at) {
                    self.rest_at_latest();
                    return true;
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
                    "PageDown" => self.page(1.0),
                    "PageUp" => self.page(-1.0),
                    "End" => self.rest_at_latest(),
                    "ArrowDown" => self.move_focus(1),
                    "ArrowUp" => self.move_focus(-1),
                    _ => return false,
                }
                return true;
            }
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
    use ambre_draw::kinetic::LONG_PRESS_MS;
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
        /// What the view said to a screen reader, oldest first.
        said: Rc<RefCell<Vec<String>>>,
        /// Each target the view handed the page, oldest first.
        acted: Rc<RefCell<Vec<Option<MessageTarget>>>>,
    }

    fn page() -> Page {
        let (events, from_logic) = local(64);
        let (to_logic, commands) = local(64);
        let pane = Rc::new(Cell::new(MurmurePane::default()));
        let shown = Rc::clone(&pane);
        let clock = Rc::new(Cell::new(0.0));
        let now = Rc::clone(&clock);
        let said_log = Rc::new(RefCell::new(Vec::new()));
        let said = Rc::clone(&said_log);
        let acted_log = Rc::new(RefCell::new(Vec::new()));
        let acted = Rc::clone(&acted_log);
        let reads = PageReads {
            pane: Rc::new(move || shown.get()),
            name_of: Rc::new(|_| Some("Ada".to_owned())),
            time_of: Rc::new(|_| "t".to_owned()),
            now_ms: Rc::new(move || now.get()),
            announce: Rc::new(move |text: &str| said.borrow_mut().push(text.to_owned())),
            act_on: Rc::new(move |target| acted.borrow_mut().push(target)),
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
            said: said_log,
            acted: acted_log,
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

    /// A key pressed at `now_ms`, with `ctrl` held or not.
    fn key(page: &mut Page, key: &str, ctrl: bool, now_ms: f64) -> bool {
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
        assert!(key(&mut paged, "PageUp", false, 10.0));
        assert!(wheel(&mut wheeled, -1.0, 2, 10.0));
        frame_at(&mut paged.view, 10.0);
        frame_at(&mut wheeled.view, 10.0);
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

        assert!(key(&mut paged, "PageDown", false, 20.0));
        frame_at(&mut paged.view, 20.0);
        assert!(shown(&paged.view).latest_shown);
        assert!(
            !key(&mut paged, "PageUp", true, 30.0),
            "the browser's chord"
        );
        assert!(!key(&mut paged, "a", false, 30.0));
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
            key(page, "PageUp", false, 10.0);
            frame(&mut page.view);
            assert!(!shown(&page.view).latest_shown);
        };

        up(&mut page);
        assert!(key(&mut page, "End", false, 20.0));
        frame(&mut page.view);
        assert!(shown(&page.view).latest_shown);
        assert_eq!(page.view.resting_on(), None);
        assert_eq!(page.view.jump, None);

        up(&mut page);
        let at = centre(&page.view);
        let click = InputEvent::PointerUp {
            x: at.x,
            y: at.y,
            button: 0,
            ctrl: false,
        };
        assert!(page.view.handle_input(&click), "the click is taken");
        frame(&mut page.view);
        assert!(shown(&page.view).latest_shown);

        up(&mut page);
        let at = centre(&page.view);
        page.view.touch_down(at.x as f32, at.y as f32, 100.0);
        assert_eq!(page.view.touch_up(150.0), None, "the tap is taken");
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
            key(&mut page, "PageUp", false, now);
            frame_at(&mut page.view, now);
        }
        segment(&page, channel, 5_000, 1);
        key(&mut page, "PageDown", false, 40.0);
        frame_at(&mut page.view, 40.0);
        assert!(!shown(&page.view).latest_shown);
        assert_eq!(
            reads(&page).0,
            0,
            "the page stopped short of the new message"
        );
        key(&mut page, "End", false, 50.0);
        assert_eq!(reads(&page).0, 0, "nothing is read before it paints");
        frame_at(&mut page.view, 50.0);
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
        assert!(key(&mut page, "ArrowUp", false, 10.0), "the key is taken");
        assert_eq!(
            page.view.focused(),
            row(&rows, 399),
            "the last row on screen"
        );
        assert_eq!(
            page.said.borrow().last().map(String::as_str),
            Some("Ada, t: hello")
        );
        key(&mut page, "ArrowUp", false, 20.0);
        assert_eq!(page.view.focused(), row(&rows, 398));
        key(&mut page, "ArrowDown", false, 30.0);
        key(&mut page, "ArrowDown", false, 40.0);
        assert_eq!(
            page.view.focused(),
            row(&rows, 399),
            "the latest is the end"
        );
        assert_eq!(page.said.borrow().len(), 4, "each move is said");
        for step in 0..60 {
            key(&mut page, "ArrowUp", false, 50.0 + f64::from(step));
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
            key(&mut page, "PageUp", false, now);
            frame_at(&mut page.view, now);
        }
        segment(&page, channel, 5_000, 1);
        for (step, arrow) in ["ArrowUp", "ArrowUp", "ArrowDown", "ArrowDown", "ArrowDown"]
            .into_iter()
            .enumerate()
        {
            key(&mut page, arrow, false, 30.0 + step as f64);
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
        key(&mut page, "ArrowUp", false, 10.0);
        frame(&mut page.view);
        let focus = page.view.focused();
        page.said.borrow_mut().clear();
        segment(&page, channel, 5_000, 1);
        frame(&mut page.view);
        assert_eq!(*page.said.borrow(), ["Ada, t: hello"]);
        assert_eq!(page.view.focused(), focus, "the focus did not move");
        assert_eq!(page.view.resting_on(), None);

        key(&mut page, "PageUp", false, 20.0);
        frame_at(&mut page.view, 20.0);
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
        key(&mut page, "PageUp", false, 10.0);
        frame_at(&mut page.view, 10.0);
        key(&mut page, "PageUp", false, 20.0);
        frame_at(&mut page.view, 20.0);
        assert_eq!(*page.said.borrow(), [JUMP_HINT]);
        assert!(JUMP_HINT.contains("End"));
        assert!(key(&mut page, "End", false, 30.0));
        frame_at(&mut page.view, 30.0);
        key(&mut page, "PageUp", false, 40.0);
        frame_at(&mut page.view, 40.0);
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
        let root = message(ada, 0, "root");
        let segment = SegmentId::new();
        let messages = vec![root.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let thread = DocumentId::new();
        let replies: Vec<_> = (1..=200).map(|ts| message(ada, ts, "hello")).collect();
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
        let press = |view: &mut MurmureView, key: &str, now_ms: f64| {
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
        assert!(press(&mut view, "ArrowUp", 10.0));
        assert_eq!(view.focused(), row(&replies, 199), "the last reply");
        assert_eq!(*page.said.borrow(), ["Ada, t: hello"]);
        assert!(press(&mut view, "PageUp", 20.0));
        press(&mut view, "PageUp", 30.0);
        assert!(!shown(&view).latest_shown, "the pages moved the thread");
        assert_eq!(
            page.said.borrow().last().map(String::as_str),
            Some(JUMP_HINT)
        );
        assert!(press(&mut view, "End", 40.0));
        assert!(shown(&view).latest_shown, "End is back at the latest reply");
        assert_eq!(page.view.focused(), None, "the channel kept its own focus");

        page.said.borrow_mut().clear();
        let ev = MurmureEvent::ThreadMessages {
            root: root.id,
            thread,
            messages: vec![message(ada, 500, "hello")],
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

    fn button(page: &mut Page, button: i16, y: f32) -> bool {
        let ev = InputEvent::PointerDown {
            x: 10.0,
            y: y.into(),
            button,
            shift: false,
            ctrl: false,
        };
        page.view.handle_input(&ev)
    }

    /// A right-click on a message hands the page that message, where it lies
    /// and its text (`MURMURE-422`). One off any message hands none, and a
    /// left click hands nothing. No command leaves (`MURMURE-424`).
    #[test]
    fn a_right_click_hands_the_page_its_message_and_sends_nothing() {
        let mut page = page();
        let channel = ChannelId::new();
        let segment = list(&page, channel, &["a", "b"]);
        page.pane.set(MurmurePane::channel(channel));
        frame(&mut page.view);
        let _ = reads(&page);
        let rows = page
            .view
            .lists
            .borrow()
            .conversations
            .channel(channel)
            .map(|l| l.keys());
        let Some(RowKey::Message(second)) = rows.and_then(|keys| keys.get(1).copied()) else {
            panic!("two messages listed");
        };

        let y = middle_of(&page.view, 1);
        assert!(!button(&mut page, 2, y));
        let target = MessageTarget {
            channel,
            at: MessageAt::Segment(segment),
            message: second,
            text: "b".to_owned(),
            notice: false,
            pressed: None,
            canvas: CHANNEL_CANVAS,
        };
        assert_eq!(page.acted.borrow().as_slice(), [Some(target)]);
        // The list rests on its latest row: the top of the region is empty.
        let above = shown(&page.view).rows.first().map_or(0.0, |r| r.1) - 1.0;
        assert!(above > 0.0, "room above the first row");
        button(&mut page, 2, above);
        assert_eq!(page.acted.borrow().last(), Some(&None), "off any message");
        button(&mut page, 0, y);
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
        let root = message(ada, 0, "root");
        let segment = SegmentId::new();
        let messages = vec![root.clone()];
        send(&page, channel, MurmureEvent::Messages { segment, messages });
        let reply = message(ada, 1, "one");
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
        thread_view.touch_down(10.0, y, 0.0);
        frame_at(&mut thread_view, LONG_PRESS_MS - 1.0);
        assert!(page.acted.borrow().is_empty(), "not yet held");
        frame_at(&mut thread_view, LONG_PRESS_MS);
        let target = MessageTarget {
            channel,
            at: MessageAt::Thread(root.id),
            message: reply.id,
            text: "one".to_owned(),
            notice: false,
            pressed: Some((10.0, y)),
            canvas: THREAD_CANVAS,
        };
        assert_eq!(page.acted.borrow().as_slice(), [Some(target)]);
        assert_eq!(thread_view.touch_up(LONG_PRESS_MS + 100.0), None, "no tap");
        frame_at(&mut thread_view, LONG_PRESS_MS + 200.0);
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
        assert!(key(&mut page, "ContextMenu", false, 10.0));
        assert_eq!(page.acted.borrow().as_slice(), [None], "no focus yet");

        assert!(key(&mut page, "ArrowUp", false, 20.0));
        frame(&mut page.view);
        let (_, bottom) = shown(&page.view).focus.expect("the focus on screen");
        // The paints at the bottom marked the latest message read.
        while page.commands.try_recv().is_ok() {}
        assert!(key(&mut page, "ContextMenu", false, 30.0));
        let shift_f10 = InputEvent::Key {
            key: "F10".to_owned(),
            ctrl: false,
            alt: false,
            shift: true,
            meta: false,
        };
        assert!(page.view.handle_input(&shift_f10));
        assert!(
            !key(&mut page, "F10", false, 40.0),
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
