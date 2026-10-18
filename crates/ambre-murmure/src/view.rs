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
use ambre_bus::{DocumentId, LogicCommand, Timestamp, UiEvent, UserId};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache, Theme,
    ThemeControl,
};
use ambre_draw::{DrawSurface, FontContext, Point, Rect};

use crate::MurmurePane;
use crate::list::{Conversations, MessageList, ReadMarks, UnreadLine};
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

    pub fn plan_repaint(&mut self, _now_ms: f64, _hud_tick: bool) -> RepaintPlan {
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.dirty = true;
        }
        let shown = ((self.reads.pane)(), self.lists.borrow().revision);
        if shown != self.painted {
            self.painted = shown;
            self.dirty = true;
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
        let painted = paint_list(
            surface,
            font_cx,
            text_cache,
            Shown { list, unread },
            self.size,
            self.theme.palette(),
            &*self.reads.name_of,
            &*self.reads.time_of,
        );
        self.last = Some(painted);
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
        if let (Some(command), Some(to_logic)) = (command, &self.to_logic)
            && let Err(e) = to_logic.try_send(command)
        {
            tracing::warn!("Read cursor dropped, Logic channel full ({e:?})");
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

    /// The region follows the pointer. Nothing on it consumes an event yet.
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        match *ev {
            InputEvent::PointerDown { x, y, .. }
            | InputEvent::PointerMove { x, y }
            | InputEvent::PointerUp { x, y, .. } => self.hover = Some(Point::new(x, y)),
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
    use ambre_bus::{ChannelId, MurmureCommand, MurmureEvent, SegmentId};
    use ambre_draw::testing::RecordingSurface;

    use super::*;
    use crate::MessageRef;
    use crate::list::tests::message;

    struct Page {
        view: MurmureView,
        pane: Rc<Cell<MurmurePane>>,
        events: Sender<UiEvent>,
        commands: Receiver<LogicCommand>,
    }

    fn page() -> Page {
        let (events, from_logic) = local(64);
        let (to_logic, commands) = local(64);
        let pane = Rc::new(Cell::new(MurmurePane::default()));
        let shown = Rc::clone(&pane);
        let reads = PageReads {
            pane: Rc::new(move || shown.get()),
            name_of: Rc::new(|_| Some("Ada".to_owned())),
            time_of: Rc::new(|_| "t".to_owned()),
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
            events,
            commands,
        }
    }

    /// Plan a frame and paint it when the plan asks, as the frame loop does.
    fn frame(view: &mut MurmureView) -> bool {
        view.update();
        let plan = view.plan_repaint(0.0, false);
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
