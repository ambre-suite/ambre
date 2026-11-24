// ambre-app/src/host/prose.rs
// ============================================================================
// The Prose view on the canvas (`features/ambre-spec-prose-page-rendering.md`).
//
// A thin host over `ambre_prose::ui::ProseUiState`. It drains the Logic->UI
// channel into that state, fills the page palette from the theme tokens, and
// paints the pages. The layout arrives computed and shaped (`PROSE-UI-006`):
// nothing here lays text out. The five figures go to the shared status bar
// slot (`PROSE-FEAT-040`); Prose builds no bar of its own (`PROSE-FEAT-123`).
//
// Keys, presses and typed text are translated into `ProseInput` and handed to
// the state, with the one send path (`PROSE-UI-002`). That path is the lossless
// outbox Toile and Trame send their edits through (`BUS-011`). The draft is shaped here,
// on this thread's font context, and nothing else is (`PROSE-UI-134`).
//
// The comment panel and the suggestion popover are DOM chrome (`PROSE-UI-050`,
// `PROSE-UI-052`). The view publishes where they go, in CSS pixels, and drains
// what they ask for once a frame. The format bar is the same. The view
// publishes where it goes and what its controls report. It drains the
// formatting id a control or a chord left in its signal (`PROSE-UI-056`).
// ============================================================================

use leptos::prelude::*;
use parley::{FontContext, LayoutContext};

use ambre_bus::{Feed, Outbox, PasteSource, Port, SendOutcome, UiEvent};
use ambre_canvas::theme::{Theme, ThemeControl};
use ambre_canvas::{
    BusyLevel, BusyPublisher, ContentKind, ContentRepaint, FrameStats, InputEvent, LayerRepaint,
    NoticePublisher, RepaintPlan, TextCache, TextInput, WgpuSurface,
};
use ambre_chrome::commands::{ClipboardWrite, ModifierKeys};
use ambre_core::Flag as _;
use ambre_core::id::{Document, Id};
use ambre_core::{Authored, Instant, PageLoad};
use ambre_core::{CanvasCss, Css, Point, Px, Size, View};
use ambre_draw::{Consumed, DrawSurface, ParleyBrush, Rect};
pub use ambre_prose::chrome::ProseSignals;
use ambre_prose::events::BlockId;
use ambre_prose::input::{ProseInput, ProseKey};
use ambre_prose::message::{ProseCommand, ProseEvent, WordCount};
use ambre_prose::probe::{Answer, Dispatch, KeyProbe, Measure, ProbeName};
use ambre_prose::ui::{ProseUiState, send_edit, send_request, status_figures};
use ambre_prose::view::paint::PagePalette;

/// The page palette, read from the theme tokens (`PROSE-UI-014`).
///
/// The sheet is the content surface. The accent and the three statuses are
/// the chrome's own, so a page and the control beside it agree.
pub(crate) fn page_palette(theme: Theme) -> PagePalette {
    let canvas = theme.palette();
    let chrome = theme.chrome_palette();
    PagePalette {
        sheet: canvas.content_bg,
        sheet_border: canvas.grid_line,
        shadow: canvas.header_border,
        text: canvas.cell_text,
        accent: chrome.accent,
        warning: chrome.warning,
        success: chrome.success,
        error: chrome.error,
    }
}

/// Where the canvas's top-left corner sits on the page. The chrome moves the
/// view's positions onto the page by it. A canvas pixel is a CSS pixel
/// (`CANVAS-032`), so the offset is the whole conversion.
fn canvas_origin() -> Point<Px<Css>> {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".ambre-canvas").ok().flatten())
        .map_or_else(Point::default, |el| {
            let r = el.get_bounding_client_rect();
            Point::new(
                Px::<Css>::new(r.left() as f32),
                Px::<Css>::new(r.top() as f32),
            )
        })
}

/// One open Prose document on the canvas.
pub struct ProseView {
    document_id: Id<Document>,
    /// The one path out: a full channel holds an edit here (`BUS-011`), and
    /// drops a request the page asks again (`BUS-023`).
    outbox: Outbox<ProseCommand>,
    busy_pub: BusyPublisher,
    /// What the shell shows the user when the page cannot, such as a reply
    /// lost with its thread (`PROSE-UI-232`).
    notices: NoticePublisher,
    from_logic: Feed<ProseEvent>,
    state: ProseUiState,
    theme_ctl: ThemeControl,
    theme: Theme,
    size: (Px<View>, Px<View>),
    /// Something visible changed since the last content paint.
    dirty: bool,
    /// The shared pipe, whose live region carries the announcements.
    text_input: TextInput,
    /// The block the last announcement named.
    announced: Option<BlockId>,
    fatal: bool,
    /// The five figures as the status bar slot shows them.
    figures: RwSignal<Option<ambre_prose::ui::StatusFigures>>,
    shown: Option<WordCount>,
    /// The zoom chip of the status bar: the chip writes a factor, the view
    /// applies it and writes back what it shows (`CHROME-206`).
    zoom: RwSignal<ambre_core::Scale<ambre_core::Doc, View>>,
    /// The factor the chip last read from the view. A signal that differs
    /// from it holds a factor the chip asked for.
    zoom_shown: ambre_core::Scale<ambre_core::Doc, View>,
    annotations: ProseSignals,
    /// The layout context the draft is shaped with.
    layout_cx: LayoutContext<ParleyBrush>,
    /// The clock of the last frame, to count presses in place.
    now: Instant<PageLoad>,
    /// The keystrokes in flight, from their input to their frame.
    probe: KeyProbe,
    timeline: super::probe::Timeline,
    /// The last pointer over the canvas, the point a `Ctrl` wheel zooms
    /// about (`ZOOM-003`).
    pointer: Option<Point<Px<View>>>,
}

/// The width and the height of a canvas box, as the view pixels the page
/// lays out against. A view pixel is a CSS pixel in size (`CANVAS-032`).
fn view_size(size: Size<Px<CanvasCss>>) -> (Px<View>, Px<View>) {
    (
        Px::<View>::new(size.width.get() as f32),
        Px::<View>::new(size.height.get() as f32),
    )
}

/// The wiring of a page view: its channels to the Logic thread, its text
/// pipe, its publishers and its chrome signals. The launcher builds the parts
/// and fills one per view.
pub struct ProseWiring {
    /// The queue to the Logic thread.
    pub to_logic: Port<ProseCommand>,
    /// The queue from the Logic thread.
    pub from_logic: Feed<ProseEvent>,
    /// The shared text pipe.
    pub text_input: TextInput,
    /// The active theme.
    pub theme_ctl: ThemeControl,
    /// The status bar slot the view fills with its figures.
    pub figures: RwSignal<Option<ambre_prose::ui::StatusFigures>>,
    /// The zoom chip of the status bar slot.
    pub zoom: RwSignal<ambre_core::Scale<ambre_core::Doc, View>>,
    /// The annotation signals the view shares with the shell.
    pub annotations: ProseSignals,
    /// The level a full outbox raises.
    pub busy_pub: BusyPublisher,
    /// What the page must tell the user and cannot show on the page.
    pub notices: NoticePublisher,
}

impl ProseView {
    /// A view of `document_id` in a canvas box of `size`, wired by `wiring`.
    pub fn new(document_id: Id<Document>, size: Size<Px<CanvasCss>>, wiring: ProseWiring) -> Self {
        let ProseWiring {
            to_logic,
            from_logic,
            text_input,
            theme_ctl,
            figures,
            zoom,
            annotations,
            busy_pub,
            notices,
        } = wiring;
        // A page is never embedded, so it always owns the pipe it is handed.
        text_input.name_content(ContentKind::Page);
        let theme = theme_ctl.get();
        let size = view_size(size);
        let mut state = ProseUiState::default();
        state.resize(size.0, size.1);
        let zoom_shown = state.viewport.zoom;
        Self {
            document_id,
            outbox: Outbox::new(to_logic),
            busy_pub,
            notices,
            from_logic,
            state,
            theme_ctl,
            theme,
            size,
            dirty: true,
            text_input,
            announced: None,
            fatal: false,
            figures,
            shown: None,
            zoom,
            zoom_shown,
            annotations,
            layout_cx: LayoutContext::new(),
            now: Instant::<PageLoad>::new(0.0),
            probe: KeyProbe::default(),
            timeline: super::probe::Timeline::new(),
            pointer: None,
        }
    }

    fn signal_fatal(&mut self, message: &str) {
        if !self.fatal {
            self.fatal = true;
            ambre_canvas::signal_fatal(message);
        }
    }

    pub fn update(&mut self) {
        self.probe.frame();
        loop {
            match self.from_logic.try_recv() {
                Ok(
                    UiEvent::Viewport { document_id, event }
                    | UiEvent::Presence { document_id, event },
                ) if document_id == self.document_id => {
                    let answer = if ambre_prose::probe::lays_out(&event) {
                        Answer::Layout
                    } else {
                        Answer::Other
                    };
                    self.state.apply_event(&event);
                    let answered =
                        self.probe
                            .answered(super::probe::now(), &self.state.draft, answer);
                    self.timeline.record(answered);
                    self.timeline
                        .count(ProbeName::ConfirmFrames, self.probe.take_counts());
                    if let Some(file) = self.state.take_export() {
                        download(&file.filename, &file.media_type, &file.bytes);
                    }
                    self.dirty = true;
                }
                Ok(UiEvent::FatalError { cause }) => self.signal_fatal(&cause.to_string()),
                Ok(_) => {}
                Err(ambre_bus::conduit::TryRecvError::Empty) => break,
                Err(ambre_bus::conduit::TryRecvError::Disconnected) => {
                    self.signal_fatal("Logic thread disconnected");
                    break;
                }
            }
        }
        // An accepted or rejected suggestion takes its popover along.
        self.publish_hover();
        self.drop_lost_drafts();
        self.take_intent();
        self.take_format();
        self.take_page();
        self.take_ruler();
        self.take_slash_pick();
        self.sync_zoom();
        self.publish_toolbar();
        // The input held behind a structural command goes out once the
        // Logic thread answered it, unless an edit still waits in the outbox.
        if !self.fatal && !self.outbox.is_busy() {
            let (outbox, document_id) = (&mut self.outbox, self.document_id);
            let mut send = |command| send_edit(outbox, document_id, command);
            self.dirty |= self.state.replay(&mut send).is_yes();
            self.busy_pub
                .publish(BusyLevel::from_busy(self.outbox.is_busy()));
        }
        if self.shown != Some(self.state.word_count) {
            self.shown = Some(self.state.word_count);
            self.figures
                .set(Some(status_figures(&self.state.word_count)));
        }
    }

    /// Apply the factor the zoom chip asked for, then show the chip the
    /// factor the page is drawn at. A zoom key moves the page alone, so the
    /// chip learns of it here too.
    fn sync_zoom(&mut self) {
        let asked = self.zoom.get_untracked();
        if asked != self.zoom_shown
            && self.state.zoom_to(asked) == ambre_prose::view::zoom::ZoomChange::Moved
        {
            self.dirty = true;
        }
        let shown = self.state.viewport.zoom;
        if shown != asked {
            self.zoom.set(shown);
        }
        self.zoom_shown = shown;
    }

    /// Act on what the panel or the popover asked for. An intent that waits
    /// behind a busy outbox stays in its signal for a later frame.
    fn take_intent(&mut self) {
        if self.fatal || self.outbox.is_busy() {
            return;
        }
        let Some(intent) = self.annotations.intent.get_untracked() else {
            return;
        };
        self.annotations.intent.set(None);
        let now = Instant::<Authored>::new(js_sys::Date::now() as u64);
        if let Some(command) = self.state.take_intent(intent, now) {
            send_edit(&mut self.outbox, self.document_id, command);
            self.busy_pub
                .publish(BusyLevel::from_busy(self.outbox.is_busy()));
        }
        self.publish_hover();
        self.dirty = true;
    }

    /// Drop each reply draft whose thread a peer took out of the panel, and
    /// tell the user of each that held text (`PROSE-UI-232`).
    fn drop_lost_drafts(&self) {
        for lost in self.annotations.drop_lost_drafts(&self.state) {
            self.notices.publish(lost.notice());
        }
    }

    /// Act on the formatting id a control or a chord invoked. One that waits
    /// behind a busy outbox stays in its signal for a later frame.
    fn take_format(&mut self) {
        if self.fatal || self.outbox.is_busy() {
            return;
        }
        let Some(id) = self.annotations.format.get_untracked() else {
            return;
        };
        self.annotations.format.set(None);
        if let Some(command) = self.state.format(id) {
            send_edit(&mut self.outbox, self.document_id, command);
        }
        self.busy_pub
            .publish(BusyLevel::from_busy(self.outbox.is_busy()));
    }

    /// Act on the page format a control of the context panel asked for: one
    /// command for the whole format, one undo item (`PROSE-UI-252`). One that
    /// waits behind a busy outbox stays in its signal for a later frame.
    fn take_page(&mut self) {
        if self.fatal || self.outbox.is_busy() {
            return;
        }
        let Some(command) = self.annotations.take_page_write() else {
            return;
        };
        send_edit(&mut self.outbox, self.document_id, command);
        self.busy_pub
            .publish(BusyLevel::from_busy(self.outbox.is_busy()));
    }

    /// Act on a released ruler handle: one command for the whole drag
    /// (`PROSE-UI-063`). One that waits behind a busy outbox stays in its
    /// signal for a later frame.
    fn take_ruler(&mut self) {
        if self.fatal || self.outbox.is_busy() {
            return;
        }
        let Some(release) = self.annotations.ruler_release.get_untracked() else {
            return;
        };
        self.annotations.ruler_release.set(None);
        if let Some(command) = self.state.ruler_release(release) {
            send_edit(&mut self.outbox, self.document_id, command);
        }
        self.busy_pub
            .publish(BusyLevel::from_busy(self.outbox.is_busy()));
    }

    /// Publish the ruler in pixels of the view. The release comes back in
    /// points, so the pixels it was measured in never reach the view.
    fn publish_ruler(&self) {
        let ruler = self.state.ruler();
        if self
            .annotations
            .ruler
            .with_untracked(|shown| *shown != ruler)
        {
            self.annotations.ruler.set(ruler);
        }
    }

    /// Publish where the canvas sits on the page, before any position the
    /// chrome moves onto the page by it.
    fn publish_origin(&self) {
        let origin = canvas_origin();
        if self
            .annotations
            .canvas_origin
            .with_untracked(|shown| *shown != origin)
        {
            self.annotations.canvas_origin.set(origin);
        }
    }

    /// Act on the slash menu item picked with the pointer. It goes down the
    /// input path, as Enter on the item would.
    fn take_slash_pick(&mut self) {
        if self.fatal || self.outbox.is_busy() {
            return;
        }
        let Some(index) = self.annotations.slash_pick.get_untracked() else {
            return;
        };
        self.annotations.slash_pick.set(None);
        let (outbox, document_id) = (&mut self.outbox, self.document_id);
        let mut send = |command| send_edit(outbox, document_id, command);
        self.dirty |= self
            .state
            .handle_input(&ProseInput::SlashPick(index), &mut send)
            .is_yes();
        self.busy_pub
            .publish(BusyLevel::from_busy(self.outbox.is_busy()));
    }

    /// Publish the slash menu where the view placed it.
    fn publish_slash(&self) {
        let menu = self.state.slash_menu();
        if self
            .annotations
            .slash
            .with_untracked(|shown| *shown != menu)
        {
            self.annotations.slash.set(menu);
        }
    }

    /// Publish the format bar, and ask for the marks it still lacks
    /// (`PROSE-UI-057`). A read the channel drops is asked again next frame.
    fn publish_toolbar(&mut self) {
        self.publish_origin();
        if !self.fatal
            && let Some(query) = self.state.marks_query()
            && send_request(&self.outbox, self.document_id, query).accepted()
        {
            self.state.marks_sent();
        }
        let bar = self.state.toolbar();
        if self
            .annotations
            .toolbar
            .with_untracked(|shown| *shown != bar)
        {
            self.annotations.toolbar.set(bar);
            // The words the subtitle of the panel counts move with the
            // selection, which moves the bar (`CHROME-241`).
            let words = self.state.selected_words();
            if self
                .annotations
                .selection_words
                .with_untracked(|shown| *shown != words)
            {
                self.annotations.selection_words.set(words);
            }
        }
        // The page the layout was measured on, for the context panel
        // (`PROSE-UI-253`).
        let page = Some(self.state.shaped.layout.format);
        if self.annotations.page.with_untracked(|shown| *shown != page) {
            self.annotations.page.set(page);
        }
        // The slash menu sits at the caret, and moves when the bar does.
        self.publish_slash();
        self.publish_ruler();
    }

    /// Publish the hovered suggestion, where the popover goes.
    fn publish_hover(&self) {
        let hovered = self.state.hovered;
        if self.annotations.hovered.get_untracked() != hovered {
            self.annotations.hovered.set(hovered);
        }
        // The spelling popover follows the view the same way: a pick, a
        // press elsewhere or a check that clears the word closes it.
        if self.annotations.spelling.get_untracked() != self.state.spelling {
            self.annotations.spelling.set(self.state.spelling.clone());
        }
    }

    /// Publish the threads the panel lists, lined up with their anchors.
    fn publish_threads(&self) {
        let threads = self.state.panel_threads();
        if self
            .annotations
            .threads
            .with_untracked(|shown| *shown != threads)
        {
            self.annotations.threads.set(threads);
        }
    }

    /// Offer the held edits to the channel, then publish the busy level the
    /// chrome shows and the input gate reads (`BUS-016`).
    fn flush_outbox(&mut self) {
        if self.fatal {
            return;
        }
        if self.outbox.flush() == SendOutcome::Disconnected {
            self.signal_fatal("Logic thread disconnected");
            return;
        }
        self.busy_pub
            .publish(BusyLevel::from_busy(self.outbox.is_busy()));
    }

    /// Announce the block the caret entered (`CHROME-023`). A move inside the
    /// block says nothing. A block no layout carries yet is announced once one
    /// does, on a later frame.
    fn announce_caret_block(&mut self) {
        let at = self.state.cursor.map(|c| c.block_id);
        if at == self.announced {
            return;
        }
        match self.state.caret_announcement() {
            Some((block, text)) => {
                self.text_input.announce(&text);
                self.announced = Some(block);
            }
            None if at.is_none() => self.announced = None,
            None => {}
        }
    }

    pub fn plan_repaint(
        &mut self,
        now: Instant<PageLoad>,
        _hud_tick: ambre_canvas::HudTick,
    ) -> RepaintPlan {
        self.now = now;
        self.timeline.summarize(now);
        self.flush_outbox();
        // A recount is owed half a second after the text last changed
        // (`PROSE-FEAT-124`). A full channel drops it, and the next change
        // asks again.
        if let Some(command) = self.state.tick(now) {
            send_request(&self.outbox, self.document_id, command);
        }
        // The pages the canvas is about to show, asked for before a frame
        // needs them (`PROSE-UI-211`). The view records the request as asked
        // before it leaves, so a full channel holds it rather than drop it.
        if let Some(command) = self.state.window_request() {
            send_edit(&mut self.outbox, self.document_id, command);
        }
        // A drag held near an edge scrolls the page (`PROSE-UI-038`), and a
        // flick keeps it moving (`PROSE-UI-217`).
        if self.state.tick_autoscroll(now) | self.state.tick_fling(now) {
            self.state.hovered = None;
            self.publish_hover();
            self.publish_toolbar();
            self.dirty = true;
        }
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.dirty = true;
        }
        self.announce_caret_block();
        let dirty = std::mem::take(&mut self.dirty);
        if dirty {
            self.publish_threads();
        }
        RepaintPlan {
            content: if dirty {
                ContentRepaint::Full
            } else {
                ContentRepaint::Reuse
            },
            content_strip: None,
            overlay: if dirty {
                LayerRepaint::Redraw
            } else {
                LayerRepaint::Reuse
            },
            backdrop: self.theme.palette().content_bg,
        }
    }

    pub fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        _text_cache: &mut TextCache,
        _strip: Option<Rect>,
    ) {
        let start = super::probe::now();
        self.state.shape_draft(font_cx, &mut self.layout_cx);
        if !self.state.draft.is_empty() {
            self.timeline.record(vec![Measure {
                name: ProbeName::Shape,
                start,
                end: super::probe::now(),
            }]);
        }
        let backdrop = Rect::new(
            0.0,
            0.0,
            f64::from(self.size.0.get()),
            f64::from(self.size.1.get()),
        );
        surface.fill_rect(backdrop, self.theme.palette().content_pending);
        self.state.paint(surface, &page_palette(self.theme));
        // The input pipe follows the caret the paint just drew, so the IME
        // popup opens beside it (`CHROME-007`).
        if let Some(bar) = self.state.caret_drawn {
            let at = Point::new(
                Px::<View>::new(bar.x0 as f32),
                Px::<View>::new(bar.y0 as f32),
            );
            let height = Px::<View>::new(bar.height().max(1.0) as f32);
            self.text_input.set_caret(at, height);
        }
        let painted = self.probe.painted(super::probe::now());
        self.timeline.record(painted);
    }

    pub fn paint_overlay(
        &mut self,
        _surface: &mut WgpuSurface<'_>,
        _font_cx: &mut FontContext,
        _text_cache: &mut TextCache,
        _stats: &FrameStats,
    ) {
    }

    /// The canvas now lays out at `size` logical pixels (`CANVAS-044`).
    pub fn resize(&mut self, size: Size<Px<CanvasCss>>) {
        self.size = view_size(size);
        self.state.resize(self.size.0, self.size.1);
        self.dirty = true;
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> Consumed {
        // A press in the page gives the keyboard focus to the pipe, so the
        // next keystroke reaches the document (`ambre-spec-text-input.md`,
        // section 3). The canvas cannot take it, and the pipe takes no click.
        if matches!(ev, InputEvent::PointerDown { .. }) {
            self.text_input.focus();
        }
        // An edit waits in the outbox, so the page refuses input until it
        // drains (`BUS-016`). Consuming the event is what makes the refusal
        // visible, as on the board and the grid.
        if self.outbox.is_busy() {
            return Consumed::Yes;
        }
        if let InputEvent::PointerMove { at } | InputEvent::PointerDown { at, .. } = ev {
            self.pointer = Some(*at);
        }
        match ev {
            // A wheel held with `Ctrl`, a trackpad pinch included, zooms about
            // the pointer (`ZOOM-003`, `ZOOM-004`, `PROSE-UI-128`).
            InputEvent::Wheel { delta, mods } if mods.contains(ModifierKeys::PRIMARY) => {
                let page = self.state.viewport.height.css_length();
                let line = ambre_core::Vector::new(ambre_draw::WHEEL_LINE, ambre_draw::WHEEL_LINE);
                let dy = delta.to_px(line, ambre_core::Vector::new(page, page)).dy;
                let asked = self.state.wheel_zoom(dy);
                let at = self.pointer.unwrap_or_else(|| {
                    let (width, height) = self.size;
                    Point::new(width / 2.0, height / 2.0)
                });
                if self.state.zoom_about(asked, at) == ambre_prose::view::zoom::ZoomChange::Moved {
                    self.state.hovered = None;
                    self.publish_hover();
                    self.publish_toolbar();
                    self.dirty = true;
                }
                Consumed::Yes
            }
            InputEvent::Wheel { delta, .. } => {
                // A new scroll stops the motion a flick left, and a glide
                // may leave its own (`PROSE-UI-218`, `KINETIC-002`).
                let now = web_sys::window()
                    .and_then(|w| w.performance())
                    .map_or(self.now, |p| Instant::<PageLoad>::new(p.now()));
                self.state.wheel(*delta, now);
                // The text moved from under the pointer, and the bar with it
                // (`PROSE-UI-055`).
                self.state.hovered = None;
                self.publish_hover();
                self.publish_toolbar();
                self.dirty = true;
                Consumed::Yes
            }
            _ => {
                // A primary press on an underlined word opens its spelling
                // popover and asks for its corrections (`PROSE-UI-248`).
                if let InputEvent::PointerDown {
                    at,
                    button: ambre_canvas::PointerButton::Primary,
                    ..
                } = ev
                {
                    if let Some(command) = self.state.press_spelling(*at) {
                        send_edit(&mut self.outbox, self.document_id, command);
                    }
                    self.publish_hover();
                }
                if let InputEvent::PointerMove { at } = ev {
                    if self.state.hover(*at) {
                        self.publish_hover();
                    }
                    let over_link = ambre_prose::chrome::LinkHover::from_bool(
                        self.state.link_at(*at).is_some(),
                    );
                    if self.annotations.over_link.get_untracked() != over_link {
                        self.annotations.over_link.set(over_link);
                    }
                }
                let Some(input) = self.translate(ev) else {
                    return Consumed::No;
                };
                let start = super::probe::now();
                let (outbox, document_id) = (&mut self.outbox, self.document_id);
                let mut sent = false;
                let mut send = |command| {
                    sent = true;
                    send_edit(outbox, document_id, command)
                };
                let consumed = self.state.handle_input(&input, &mut send);
                // A key or typed text starts the three keystroke measures.
                if consumed.is_yes()
                    && matches!(input, ProseInput::Key { .. } | ProseInput::Text(_))
                {
                    let dispatch = if sent { Dispatch::Sent } else { Dispatch::Kept };
                    let echoed =
                        self.probe
                            .handled(start, super::probe::now(), &self.state.draft, dispatch);
                    self.timeline.record(echoed);
                }
                // The shell's one open-URL path takes the target
                // (`PROSE-UI-071`).
                if let Some(url) = self.state.followed.take() {
                    self.annotations.followed.set(Some(url));
                }
                // The shell opens the page menu where a menu key placed it
                // (`PROSE-UI-237`).
                if let Some(at) = self.state.menu_at.take() {
                    self.annotations.menu.set(Some(at));
                }
                self.busy_pub
                    .publish(BusyLevel::from_busy(self.outbox.is_busy()));
                self.publish_toolbar();
                self.dirty |= consumed.is_yes();
                consumed
            }
        }
    }

    /// The input a canvas event stands for, or `None` for one the view leaves
    /// to the host.
    fn translate(&self, ev: &InputEvent) -> Option<ProseInput> {
        Some(match ev {
            InputEvent::Key { key, mods } if !mods.contains(ModifierKeys::ALT) => ProseInput::Key {
                key: ProseKey::from_key(*key, *mods)?,
                mods: *mods,
            },
            InputEvent::Text { text } => ProseInput::Text(text.clone()),
            InputEvent::CompositionUpdate { text } => ProseInput::Compose(text.clone()),
            InputEvent::CompositionEnd { text } => ProseInput::ComposeEnd(text.clone()),
            InputEvent::PointerDown {
                at,
                button: ambre_canvas::PointerButton::Primary,
                mods,
            } => ProseInput::Press {
                at: *at,
                // The host folds `Cmd` into `primary`; a press reads no Alt.
                mods: mods.without(ModifierKeys::ALT),
                now: self.now,
            },
            InputEvent::PointerMove { at } => ProseInput::Drag { at: *at },
            InputEvent::PointerUp {
                button: ambre_canvas::PointerButton::Primary,
                ..
            } => ProseInput::Release,
            _ => return None,
        })
    }

    /// One input the host builds itself rather than translates.
    fn feed(&mut self, input: &ProseInput) -> Consumed {
        if self.fatal || self.outbox.is_busy() {
            return Consumed::No;
        }
        let (outbox, document_id) = (&mut self.outbox, self.document_id);
        let mut send = |command| send_edit(outbox, document_id, command);
        let consumed = self.state.handle_input(input, &mut send);
        self.busy_pub
            .publish(BusyLevel::from_busy(self.outbox.is_busy()));
        self.publish_toolbar();
        self.dirty |= consumed.is_yes();
        consumed
    }

    /// A copy or a cut: the Logic thread builds the payload of the whole
    /// selection, and a frame later [`poll_clipboard`](Self::poll_clipboard)
    /// hands it over (`PROSE-UI-040`, `PROSE-UI-041`). With nothing selected
    /// the browser keeps its default.
    pub fn clipboard_write(&mut self, write: ClipboardWrite) -> ambre_bus::CopyOutcome {
        if self.state.selection().is_none() {
            return ambre_bus::CopyOutcome::Nothing;
        }
        if self.feed(&ProseInput::Copy(write)).is_yes() {
            ambre_bus::CopyOutcome::Deferred
        } else {
            ambre_bus::CopyOutcome::Nothing
        }
    }

    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        self.state.poll_clipboard()
    }

    /// A paste of what the host read, or of the payload kept from the last
    /// copy while the clipboard is still that copy (`PROSE-UI-043`).
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, source: PasteSource) {
        let input = self.state.paste_input(blob, source);
        let start = super::probe::now();
        if self.feed(&input).is_yes() {
            let echoed = self.probe.handled(
                start,
                super::probe::now(),
                &self.state.draft,
                Dispatch::Sent,
            );
            self.timeline.record(echoed);
        }
    }

    /// A finger on the canvas: a pan scrolls the column and a flick keeps it
    /// moving (`PROSE-UI-217`). A tap presses where it landed.
    pub fn touch(
        &mut self,
        phase: super::TouchPhase,
        at: Point<Px<View>>,
        now: Instant<PageLoad>,
    ) -> Consumed {
        if self.fatal {
            return Consumed::No;
        }
        match phase {
            super::TouchPhase::Down => self.state.touch_start(at, now),
            super::TouchPhase::Move => {
                if self.state.touch_move(at, now) {
                    self.state.hovered = None;
                    self.publish_hover();
                    self.publish_toolbar();
                    self.dirty = true;
                }
            }
            super::TouchPhase::Cancel => self.state.touch_cancel(),
            super::TouchPhase::Up => {
                // The finger is the view's, so the click a tap replays
                // answers nothing of its own.
                if let Some(at) = self.state.touch_end(now) {
                    let _ = self.feed(&ProseInput::Press {
                        at,
                        mods: ModifierKeys::NONE,
                        now,
                    });
                    let _ = self.feed(&ProseInput::Release);
                }
            }
        }
        Consumed::Yes
    }

    /// Text dropped on the canvas: one insertion at the drop point
    /// (`PROSE-UI-204`).
    pub fn drop_text(&mut self, at: Point<Px<View>>, text: String) -> Consumed {
        self.feed(&ProseInput::Drop { at, text })
    }

    /// Another document of the same type: the state starts over, and the
    /// first layout of the new one arrives from the Logic thread.
    pub fn set_document(&mut self, document_id: Id<Document>) {
        self.document_id = document_id;
        self.state = ProseUiState::default();
        self.state.resize(self.size.0, self.size.1);
        self.announced = None;
        self.shown = None;
        self.publish_hover();
        self.publish_toolbar();
        self.dirty = true;
    }
}

/// Hand a file the Logic thread wrote to the browser as a download, typed with
/// its media type (`PROSE-IO-056`). A browser that refuses is logged: the
/// document itself is untouched either way.
fn download(
    filename: &ambre_client_core::ExportFilename,
    media_type: &ambre_bus::MediaType,
    data: &ambre_core::Payload<ambre_client_core::FileBytes>,
) {
    use wasm_bindgen::JsCast;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(data.as_bytes()).buffer());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(media_type.as_str());
    let url = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .and_then(|blob| web_sys::Url::create_object_url_with_blob(&blob));
    let url = match url {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!("Prose: the download could not start: {e:?}");
            return;
        }
    };
    let anchor = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.create_element("a").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok());
    if let Some(anchor) = anchor {
        anchor.set_href(&url);
        anchor.set_download(filename.as_str());
        anchor.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::LogicCommand;
    use ambre_core::Count;
    use ambre_core::index::Len;
    use ambre_core::{Px, Size};
    use ambre_prose::message::{Graphemes, Page, Paragraphs, Words};
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A pointer at canvas `(x, y)`, as the host hands it over.
    fn pointer(x: f64, y: f64) -> ambre_core::Point<ambre_core::Px<ambre_core::View>> {
        ambre_core::Point::new(
            ambre_core::Px::<ambre_core::View>::new(x as f32),
            ambre_core::Px::<ambre_core::View>::new(y as f32),
        )
    }

    /// `CHROME-102`: a press in the page gives the keyboard focus to the pipe,
    /// so the next keystroke reaches the document.
    #[wasm_bindgen_test]
    fn a_press_in_the_page_focuses_the_pipe() {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let cell = document.create_element("div").expect("a cell");
        document
            .body()
            .expect("a body")
            .append_child(&cell)
            .expect("attached");
        let (to_logic, _queue) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let (_to_ui, from_logic) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let pipe = TextInput::create(&document, &cell);
        let mut view = ProseView::new(
            Id::<Document>::mint(),
            Size::new(Px::new(800), Px::new(600)),
            ProseWiring {
                to_logic: Port::direct(to_logic),
                from_logic: Feed::direct(from_logic),
                text_input: pipe.clone(),
                theme_ctl: ThemeControl::new(Theme::Dark),
                figures: RwSignal::new(None),
                zoom: RwSignal::new(ambre_core::Scale::ONE),
                annotations: ProseSignals::new(),
                busy_pub: BusyPublisher::new(),
                notices: NoticePublisher::new(),
            },
        );
        if let Some(body) = document.body() {
            let _ = body.focus();
        }
        let _ = view.handle_input(&InputEvent::PointerDown {
            at: pointer(300.0, 200.0),
            button: ambre_canvas::PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
        let focused = document.active_element().expect("a focused element");
        assert!(
            focused.is_same_node(Some(pipe.element())),
            "{}",
            focused.tag_name()
        );
    }

    /// `ZOOM-003`, `ZOOM-004`, `PROSE-UI-128`: a wheel held with `Ctrl` over
    /// the page zooms it, and the document point under the pointer stays
    /// there. The event is consumed, so the browser does not zoom the tab.
    #[wasm_bindgen_test]
    fn a_ctrl_wheel_zooms_the_page_about_the_pointer() {
        use ambre_prose::layout::pass::PageLayout;
        use ambre_prose::view::column::{DocPos, PageColumn};
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let cell = document.create_element("div").expect("a cell");
        document
            .body()
            .expect("a body")
            .append_child(&cell)
            .expect("attached");
        let (to_logic, _queue) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let (_to_ui, from_logic) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let mut view = ProseView::new(
            Id::<Document>::mint(),
            Size::new(Px::new(800), Px::new(600)),
            ProseWiring {
                to_logic: Port::direct(to_logic),
                from_logic: Feed::direct(from_logic),
                text_input: TextInput::create(&document, &cell),
                theme_ctl: ThemeControl::new(Theme::Dark),
                figures: RwSignal::new(None),
                zoom: RwSignal::new(ambre_core::Scale::ONE),
                annotations: ProseSignals::new(),
                busy_pub: BusyPublisher::new(),
                notices: NoticePublisher::new(),
            },
        );
        view.state.column = PageColumn::of(&PageLayout {
            // Wider than the canvas at every factor the notches reach, so no
            // recentring moves the point sideways (`PROSE-UI-026`).
            page_width: Px::new(1000.0),
            page_height: Px::new(800.0),
            page_count: Len::<Page>::new(40),
            ..PageLayout::default()
        });
        view.state.viewport.scroll_y = Px::new(5_000.0);
        let at = pointer(420.0, 310.0);
        let _ = view.handle_input(&InputEvent::PointerMove { at });
        let DocPos::OnPage {
            page,
            x_in_page,
            y_in_page,
        } = view.state.column.screen_to_doc(&view.state.viewport, at)
        else {
            panic!("the pointer is over a page");
        };
        let notch = |dy: f64| InputEvent::Wheel {
            delta: ambre_canvas::WheelDelta::pixels(ambre_core::Vector::new(
                ambre_canvas::WheelAmount::new(0.0),
                ambre_canvas::WheelAmount::new(dy),
            )),
            mods: ModifierKeys::PRIMARY,
        };
        for dy in [-120.0, -120.0, -60.0, 120.0, -200.0] {
            assert!(
                view.handle_input(&notch(dy)).is_yes(),
                "the browser must not zoom the tab"
            );
            let now =
                view.state
                    .column
                    .doc_to_screen(&view.state.viewport, page, x_in_page, y_in_page);
            assert!(
                (now.y - at.y).get().abs() < 0.5,
                "{:?} drifted to {:?}",
                at.y,
                now.y
            );
            assert!(
                (now.x - at.x).get().abs() < 0.5,
                "{:?} drifted to {:?}",
                at.x,
                now.x
            );
        }
        assert!(
            view.state.viewport.zoom.get() > 1.0,
            "a wheel away from the reader zooms in"
        );
        assert!(view.dirty, "the zoomed page repaints");
        cell.remove();
    }

    /// `BUS-022`: the page takes no input while its outbox holds a command, and
    /// takes it again once the queue drains.
    #[wasm_bindgen_test]
    fn a_busy_page_takes_no_input() {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let cell = document.create_element("div").expect("a cell");
        document
            .body()
            .expect("a body")
            .append_child(&cell)
            .expect("attached");
        let (to_logic, queue) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<1>());
        let (_to_ui, from_logic) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let document_id = Id::<Document>::mint();
        let mut view = ProseView::new(
            document_id,
            Size::new(Px::new(800), Px::new(600)),
            ProseWiring {
                to_logic: Port::direct(to_logic),
                from_logic: Feed::direct(from_logic),
                text_input: TextInput::create(&document, &cell),
                theme_ctl: ThemeControl::new(Theme::Dark),
                figures: RwSignal::new(None),
                zoom: RwSignal::new(ambre_core::Scale::ONE),
                annotations: ProseSignals::new(),
                busy_pub: BusyPublisher::new(),
                notices: NoticePublisher::new(),
            },
        );
        let open = || LogicCommand::OpenDocument {
            document_id,
            doc_type: ambre_bus::DocType::Prose,
        };
        for _ in 0..2 {
            let _ = view.outbox.send(open());
        }
        assert!(view.outbox.is_busy());
        let wheel = InputEvent::Wheel {
            delta: ambre_canvas::WheelDelta::pixels(ambre_core::Vector::new(
                ambre_canvas::WheelAmount::new(0.0),
                ambre_canvas::WheelAmount::new(40.0),
            )),
            mods: ModifierKeys::NONE,
        };
        view.dirty = false;
        assert!(
            view.handle_input(&wheel).is_yes(),
            "the refusal consumes the event"
        );
        assert!(!view.dirty, "the busy page scrolled");
        queue.try_iter().for_each(drop);
        assert!(matches!(view.outbox.flush(), SendOutcome::Sent));
        assert!(view.handle_input(&wheel).is_yes());
        assert!(view.dirty, "the drained page takes input again");
    }

    /// `PROSE-FEAT-040`: the page's count reaches the status bar slot every
    /// application shares, as its five figures, after one frame.
    #[wasm_bindgen_test]
    async fn the_page_count_reaches_the_shared_status_slot() {
        use crate::shell::prose_status::ProseFigures;
        use leptos::mount::mount_to;
        use leptos::wasm_bindgen::JsCast;

        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let cell = document.create_element("div").expect("a cell");
        let slot = document.create_element("div").expect("a slot");
        let body = document.body().expect("a body");
        body.append_child(&cell).expect("attached");
        body.append_child(&slot).expect("attached");
        let figures = RwSignal::new(None);
        let zoom = RwSignal::new(ambre_core::Scale::ONE);
        let target: web_sys::HtmlElement = slot.clone().dyn_into().expect("an element");
        mount_to(
            target,
            move || view! { <ProseFigures figures=figures zoom=zoom /> },
        )
        .forget();

        let (to_logic, _queue) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let (_to_ui, from_logic) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let mut view = ProseView::new(
            Id::<Document>::mint(),
            Size::new(Px::new(800), Px::new(600)),
            ProseWiring {
                to_logic: Port::direct(to_logic),
                from_logic: Feed::direct(from_logic),
                text_input: TextInput::create(&document, &cell),
                theme_ctl: ThemeControl::new(Theme::Dark),
                figures,
                zoom,
                annotations: ProseSignals::new(),
                busy_pub: BusyPublisher::new(),
                notices: NoticePublisher::new(),
            },
        );
        // The count as the page state holds it once the Logic thread's event
        // is applied. Decoding that event is Prose's own, and tested there.
        view.state.word_count = WordCount {
            words: Count::<Words>::new(12),
            characters: Count::<Graphemes>::new(60),
            characters_with_spaces: Count::<Graphemes>::new(71),
            paragraphs: Count::<Paragraphs>::new(1),
            pages: Len::<Page>::new(2),
        };
        view.update();
        leptos::task::tick().await;

        let stats = slot.query_selector_all(".stat").expect("a query");
        let shown: Vec<String> = (0..stats.length())
            .filter_map(|i| stats.item(i))
            .filter_map(|n| n.text_content())
            .collect();
        assert_eq!(
            shown,
            [
                "12 words",
                "60 characters",
                "71 with spaces",
                "1 paragraph",
                "2 pages"
            ]
        );
        cell.remove();
        slot.remove();
    }

    /// `CHROME-206`, `PROSE-UI-250`: the Prose band ends on the shared zoom
    /// chip. A preset picked there zooms the page, and a zoom key moves the
    /// chip.
    #[wasm_bindgen_test]
    async fn the_zoom_chip_closes_the_band_and_zooms_the_page() {
        use crate::shell::prose_status::ProseFigures;
        use leptos::mount::mount_to;
        use leptos::wasm_bindgen::JsCast;

        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let cell = document.create_element("div").expect("a cell");
        let slot = document.create_element("div").expect("a slot");
        let body = document.body().expect("a body");
        body.append_child(&cell).expect("attached");
        body.append_child(&slot).expect("attached");
        let figures = RwSignal::new(None);
        let zoom = RwSignal::new(ambre_core::Scale::ONE);
        let target: web_sys::HtmlElement = slot.clone().dyn_into().expect("an element");
        mount_to(
            target,
            move || view! { <ProseFigures figures=figures zoom=zoom /> },
        )
        .forget();
        let last = slot.last_element_child().expect("a band child");
        assert!(
            last.class_list().contains("statusbar-zoom"),
            "the band ends on `{}`",
            last.class_name()
        );

        let (to_logic, _queue) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let (_to_ui, from_logic) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<4>());
        let mut view = ProseView::new(
            Id::<Document>::mint(),
            Size::new(Px::new(800), Px::new(600)),
            ProseWiring {
                to_logic: Port::direct(to_logic),
                from_logic: Feed::direct(from_logic),
                text_input: TextInput::create(&document, &cell),
                theme_ctl: ThemeControl::new(Theme::Dark),
                figures,
                zoom,
                annotations: ProseSignals::new(),
                busy_pub: BusyPublisher::new(),
                notices: NoticePublisher::new(),
            },
        );
        let click = |selector: &str| {
            slot.query_selector(selector)
                .ok()
                .flatten()
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
                .expect("a control")
                .click();
        };
        click(".statusbar-zoom-btn");
        leptos::task::tick().await;
        let items = slot
            .query_selector_all(".statusbar-zoom-item")
            .expect("a query");
        let two_hundred = (0..items.length())
            .filter_map(|i| items.item(i))
            .filter_map(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
            .find(|el| el.text_content().as_deref() == Some("200 %"))
            .expect("a 200 % preset");
        view.dirty = false;
        two_hundred.click();
        view.update();
        assert_eq!(view.state.viewport.zoom.get(), 2.0);
        assert!(view.dirty, "the zoomed page repaints");

        let _ = view.handle_input(&InputEvent::Key {
            key: ambre_chrome::commands::Key::Char(ambre_chrome::commands::KeyChar::new('0')),
            mods: ModifierKeys::PRIMARY,
        });
        view.update();
        leptos::task::tick().await;
        assert_eq!(zoom.get_untracked().get(), 1.0);
        let shown = slot
            .query_selector(".statusbar-zoom-btn")
            .ok()
            .flatten()
            .and_then(|el| el.text_content());
        assert_eq!(shown.as_deref(), Some("100 %"));
        cell.remove();
        slot.remove();
    }
}
