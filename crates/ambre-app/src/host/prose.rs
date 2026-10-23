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

use ambre_bus::app::prose::WordCount;
use ambre_bus::{AppEvent, DocumentId, LogicCommand, Outbox, PasteSource, SendOutcome, UiEvent};
use ambre_canvas::theme::{Theme, ThemeControl};
use ambre_canvas::{
    BusyPublisher, ContentKind, ContentRepaint, FrameStats, InputEvent, NoticePublisher,
    OverlayRepaint, RepaintPlan, TextCache, TextInput, WgpuSurface,
};
use ambre_chrome::commands::{ClipboardWrite, Modifiers};
use ambre_draw::{DrawSurface, ParleyBrush, Rect};
pub use ambre_prose::chrome::ProseSignals;
use ambre_prose::events::BlockId;
use ambre_prose::input::{ProseInput, ProseKey};
use ambre_prose::probe::{Answer, CONFIRM_FRAMES, Dispatch, KeyProbe, Measure, SHAPE};
use ambre_prose::ui::{ProseUiState, send_command, send_edit, status_figures};
use ambre_prose::view::annotate::{HoveredSuggestion, PanelThread, ScreenRect};
use ambre_prose::view::paint::PagePalette;
use ambre_prose::view::ruler::RulerState;

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

/// From canvas pixels to CSS pixels of the window. A canvas pixel is a CSS
/// pixel (`CANVAS-032`), so only the canvas offset applies.
fn to_css(x: f32, y: f32) -> (f32, f32) {
    let (left, top) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".ambre-canvas").ok().flatten())
        .map_or((0.0, 0.0), |el| {
            let r = el.get_bounding_client_rect();
            (r.left() as f32, r.top() as f32)
        });
    (left + x, top + y)
}

/// The ruler the view published, moved into CSS pixels by `to_css`. The scale
/// keeps its ratio to the column, so each handle lands where the view put it
/// (`PROSE-UI-065`).
fn ruler_in_css(mut ruler: RulerState, to_css: impl Fn(f32, f32) -> (f32, f32)) -> RulerState {
    let (left, top) = to_css(ruler.page_left_px, 0.0);
    let (right, _) = to_css(ruler.page_left_px + ruler.content_width_px, 0.0);
    let factor = if ruler.content_width_px > 0.0 {
        (right - left) / ruler.content_width_px
    } else {
        1.0
    };
    ruler.page_left_px = left;
    ruler.top_px = top;
    ruler.content_width_px = right - left;
    ruler.px_per_pt *= factor;
    ruler
}

fn rect_to_css(rect: ScreenRect) -> ScreenRect {
    let (x0, y0) = to_css(rect.x0, rect.y0);
    let (x1, y1) = to_css(rect.x1, rect.y1);
    ScreenRect { x0, y0, x1, y1 }
}

/// One open Prose document on the canvas.
pub struct ProseView {
    document_id: DocumentId,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    /// The edits' path out: a full channel holds them here (`BUS-011`).
    outbox: Outbox,
    busy_pub: BusyPublisher,
    /// What the shell shows the user when the page cannot, such as a reply
    /// lost with its thread (`PROSE-UI-232`).
    notices: NoticePublisher,
    from_logic: ambre_bus::conduit::Receiver<UiEvent>,
    state: ProseUiState,
    theme_ctl: ThemeControl,
    theme: Theme,
    size: (f32, f32),
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
    annotations: ProseSignals,
    /// The layout context the draft is shaped with.
    layout_cx: LayoutContext<ParleyBrush>,
    /// The clock of the last frame, to count presses in place.
    now_ms: ambre_core::PageTime,
    /// The keystrokes in flight, from their input to their frame.
    probe: KeyProbe,
    timeline: super::probe::Timeline,
}

impl ProseView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_id: DocumentId,
        to_logic: ambre_bus::conduit::Sender<LogicCommand>,
        from_logic: ambre_bus::conduit::Receiver<UiEvent>,
        width: f64,
        height: f64,
        text_input: TextInput,
        theme_ctl: ThemeControl,
        figures: RwSignal<Option<ambre_prose::ui::StatusFigures>>,
        annotations: ProseSignals,
        busy_pub: BusyPublisher,
        notices: NoticePublisher,
    ) -> Self {
        // A page is never embedded, so it always owns the pipe it is handed.
        text_input.name_content(ContentKind::Page);
        let theme = theme_ctl.get();
        let size = (width as f32, height as f32);
        let mut state = ProseUiState::default();
        state.resize(size.0, size.1);
        Self {
            document_id,
            outbox: Outbox::new(to_logic.clone()),
            busy_pub,
            notices,
            to_logic,
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
            annotations,
            layout_cx: LayoutContext::new(),
            now_ms: ambre_bus::PageTime(0.0),
            probe: KeyProbe::default(),
            timeline: super::probe::Timeline::new(),
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
                    UiEvent::Viewport {
                        document_id,
                        event: AppEvent::Prose(event),
                    }
                    | UiEvent::Presence {
                        document_id,
                        event: AppEvent::Prose(event),
                    },
                ) if document_id == self.document_id => {
                    let answer = if ambre_prose::probe::lays_out(&event) {
                        Answer::Layout
                    } else {
                        Answer::Other
                    };
                    self.state.apply_event(&event);
                    let answered =
                        self.probe
                            .answered(super::probe::now_ms(), &self.state.draft, answer);
                    self.timeline.record(answered);
                    self.timeline
                        .count(CONFIRM_FRAMES, self.probe.take_counts());
                    if let Some((filename, media_type, data)) = self.state.take_export() {
                        download(&filename, &media_type, &data);
                    }
                    self.dirty = true;
                }
                Ok(UiEvent::FatalError { message }) => self.signal_fatal(&message),
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
        self.take_ruler();
        self.take_slash_pick();
        self.publish_toolbar();
        // The input held behind a structural command goes out once the
        // Logic thread answered it, unless an edit still waits in the outbox.
        if !self.fatal && !self.outbox.is_busy() {
            let (outbox, document_id) = (&mut self.outbox, self.document_id);
            let mut send = |command| send_edit(outbox, document_id, command);
            self.dirty |= self.state.replay(&mut send);
            self.busy_pub.publish(self.outbox.is_busy());
        }
        if self.shown != Some(self.state.word_count) {
            self.shown = Some(self.state.word_count);
            self.figures
                .set(Some(status_figures(&self.state.word_count)));
        }
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
        let now_ms = ambre_core::Timestamp(js_sys::Date::now() as u64);
        if let Some(command) = self.state.take_intent(intent, now_ms) {
            send_edit(&mut self.outbox, self.document_id, command);
            self.busy_pub.publish(self.outbox.is_busy());
        }
        self.publish_hover();
        self.dirty = true;
    }

    /// Drop each reply draft whose thread a peer took out of the panel, and
    /// tell the user of each that held text (`PROSE-UI-232`).
    fn drop_lost_drafts(&self) {
        for notice in self.annotations.drop_lost_drafts(&self.state) {
            self.notices.publish(notice);
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
        self.busy_pub.publish(self.outbox.is_busy());
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
        self.busy_pub.publish(self.outbox.is_busy());
    }

    /// Publish the ruler in CSS pixels. The release comes back in points, so
    /// the pixels it was measured in never reach the view.
    fn publish_ruler(&self) {
        let ruler = ruler_in_css(self.state.ruler(), to_css);
        if self
            .annotations
            .ruler
            .with_untracked(|shown| *shown != ruler)
        {
            self.annotations.ruler.set(ruler);
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
            .handle_input(&ProseInput::SlashPick(index), &mut send);
        self.busy_pub.publish(self.outbox.is_busy());
    }

    /// Publish the slash menu where the view placed it.
    fn publish_slash(&self) {
        let mut menu = self.state.slash_menu();
        menu.anchor = menu.anchor.map(|(x, y)| to_css(x, y));
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
        if !self.fatal
            && let Some(query) = self.state.marks_query()
            && send_command(&self.to_logic, self.document_id, query).accepted()
        {
            self.state.marks_sent();
        }
        let mut bar = self.state.toolbar();
        bar.anchor = bar.anchor.map(|(x, y)| to_css(x, y));
        if self
            .annotations
            .toolbar
            .with_untracked(|shown| *shown != bar)
        {
            self.annotations.toolbar.set(bar);
        }
        // The slash menu sits at the caret, and moves when the bar does.
        self.publish_slash();
        self.publish_ruler();
    }

    /// Publish the hovered suggestion, where the popover goes.
    fn publish_hover(&self) {
        let hovered = self.state.hovered.map(|h| HoveredSuggestion {
            rect: rect_to_css(h.rect),
            ..h
        });
        if self.annotations.hovered.get_untracked() != hovered {
            self.annotations.hovered.set(hovered);
        }
    }

    /// Publish the threads the panel lists, lined up with their anchors.
    fn publish_threads(&self) {
        let threads: Vec<PanelThread> = self
            .state
            .panel_threads()
            .into_iter()
            .map(|entry| PanelThread {
                y_px: to_css(0.0, entry.y_px).1,
                ..entry
            })
            .collect();
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
        self.busy_pub.publish(self.outbox.is_busy());
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

    pub fn plan_repaint(&mut self, now_ms: ambre_core::PageTime, _hud_tick: bool) -> RepaintPlan {
        self.now_ms = now_ms;
        self.timeline.summarize(now_ms);
        self.flush_outbox();
        // A recount is owed half a second after the text last changed
        // (`PROSE-FEAT-124`). A full channel drops it, and the next change
        // asks again.
        if let Some(command) = self.state.tick(now_ms) {
            send_command(&self.to_logic, self.document_id, command);
        }
        // The pages the canvas is about to show, asked for before a frame
        // needs them (`PROSE-UI-211`). A full channel drops the request, and
        // the next layout taken asks again.
        if let Some(command) = self.state.window_request() {
            send_command(&self.to_logic, self.document_id, command);
        }
        // A drag held near an edge scrolls the page (`PROSE-UI-038`), and a
        // flick keeps it moving (`PROSE-UI-217`).
        if self.state.tick_autoscroll(now_ms) | self.state.tick_fling(now_ms) {
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
                OverlayRepaint::Redraw
            } else {
                OverlayRepaint::Reuse
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
        let start = super::probe::now_ms();
        self.state.shape_draft(font_cx, &mut self.layout_cx);
        if !self.state.draft.is_empty() {
            self.timeline.record(vec![Measure {
                name: SHAPE,
                start,
                end: super::probe::now_ms(),
            }]);
        }
        let backdrop = Rect::new(0.0, 0.0, f64::from(self.size.0), f64::from(self.size.1));
        surface.fill_rect(backdrop, self.theme.palette().content_pending);
        self.state.paint(surface, &page_palette(self.theme));
        // The input pipe follows the caret the paint just drew, so the IME
        // popup opens beside it (`CHROME-007`).
        if let Some(bar) = self.state.caret_drawn {
            self.text_input
                .set_caret(bar.x0, bar.y0, bar.height().max(1.0));
        }
        let painted = self.probe.painted(super::probe::now_ms());
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

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width as f32, height as f32);
        self.state.resize(self.size.0, self.size.1);
        self.dirty = true;
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
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
            return true;
        }
        match ev {
            InputEvent::Wheel { delta, .. } => {
                // A new scroll stops the motion a flick left, and a glide
                // may leave its own (`PROSE-UI-218`, `KINETIC-002`).
                let now = web_sys::window()
                    .and_then(|w| w.performance())
                    .map_or(self.now_ms, |p| ambre_bus::PageTime(p.now()));
                self.state.wheel(*delta, now);
                // The text moved from under the pointer, and the bar with it
                // (`PROSE-UI-055`).
                self.state.hovered = None;
                self.publish_hover();
                self.publish_toolbar();
                self.dirty = true;
                true
            }
            _ => {
                if let InputEvent::PointerMove { x, y } = ev {
                    let (x, y) = (*x as f32, *y as f32);
                    if self.state.hover(x, y) {
                        self.publish_hover();
                    }
                    let over_link = self.state.link_at(x, y).is_some();
                    if self.annotations.over_link.get_untracked() != over_link {
                        self.annotations.over_link.set(over_link);
                    }
                }
                let Some(input) = self.translate(ev) else {
                    return false;
                };
                let start = super::probe::now_ms();
                let (outbox, document_id) = (&mut self.outbox, self.document_id);
                let mut sent = false;
                let mut send = |command| {
                    sent = true;
                    send_edit(outbox, document_id, command)
                };
                let consumed = self.state.handle_input(&input, &mut send);
                // A key or typed text starts the three keystroke measures.
                if consumed && matches!(input, ProseInput::Key { .. } | ProseInput::Text(_)) {
                    let dispatch = if sent { Dispatch::Sent } else { Dispatch::Kept };
                    let echoed = self.probe.handled(
                        start,
                        super::probe::now_ms(),
                        &self.state.draft,
                        dispatch,
                    );
                    self.timeline.record(echoed);
                }
                // The shell's one open-URL path takes the target
                // (`PROSE-UI-071`).
                if let Some(url) = self.state.followed.take() {
                    self.annotations.followed.set(Some(url));
                }
                self.busy_pub.publish(self.outbox.is_busy());
                self.publish_toolbar();
                self.dirty |= consumed;
                consumed
            }
        }
    }

    /// The input a canvas event stands for, or `None` for one the view leaves
    /// to the host.
    fn translate(&self, ev: &InputEvent) -> Option<ProseInput> {
        Some(match ev {
            InputEvent::Key {
                key,
                ctrl,
                alt,
                shift,
                meta,
            } if !alt => {
                let mods = Modifiers::from_event(*ctrl, false, *shift, *meta);
                ProseInput::Key {
                    key: ProseKey::from_name(key, mods)?,
                    mods,
                }
            }
            InputEvent::Text { text } => ProseInput::Text(text.clone()),
            InputEvent::CompositionUpdate { text } => ProseInput::Compose(text.clone()),
            InputEvent::CompositionEnd { text } => ProseInput::ComposeEnd(text.clone()),
            InputEvent::PointerDown {
                x,
                y,
                button: 0,
                shift,
                ctrl,
                ..
            } => ProseInput::Press {
                x: *x as f32,
                y: *y as f32,
                // The canvas folds `Cmd` into `ctrl`.
                mods: Modifiers::from_event(*ctrl, false, *shift, false),
                now_ms: self.now_ms,
            },
            InputEvent::PointerMove { x, y } => ProseInput::Drag {
                x: *x as f32,
                y: *y as f32,
            },
            InputEvent::PointerUp { button: 0, .. } => ProseInput::Release,
            _ => return None,
        })
    }

    /// One input the host builds itself rather than translates.
    fn feed(&mut self, input: &ProseInput) -> bool {
        if self.fatal || self.outbox.is_busy() {
            return false;
        }
        let (outbox, document_id) = (&mut self.outbox, self.document_id);
        let mut send = |command| send_edit(outbox, document_id, command);
        let consumed = self.state.handle_input(input, &mut send);
        self.busy_pub.publish(self.outbox.is_busy());
        self.publish_toolbar();
        self.dirty |= consumed;
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
        if self.feed(&ProseInput::Copy(write)) {
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
        let start = super::probe::now_ms();
        if self.feed(&input) {
            let echoed = self.probe.handled(
                start,
                super::probe::now_ms(),
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
        x: f32,
        y: f32,
        now_ms: ambre_core::PageTime,
    ) -> bool {
        if self.fatal {
            return false;
        }
        match phase {
            super::TouchPhase::Down => self.state.touch_start(x, y, now_ms),
            super::TouchPhase::Move => {
                if self.state.touch_move(x, y, now_ms) {
                    self.state.hovered = None;
                    self.publish_hover();
                    self.publish_toolbar();
                    self.dirty = true;
                }
            }
            super::TouchPhase::Cancel => self.state.touch_cancel(),
            super::TouchPhase::Up => {
                if let Some((x, y)) = self.state.touch_end(now_ms) {
                    self.feed(&ProseInput::Press {
                        x,
                        y,
                        mods: Modifiers::NONE,
                        now_ms,
                    });
                    self.feed(&ProseInput::Release);
                }
            }
        }
        true
    }

    /// Text dropped on the canvas: one insertion at the drop point
    /// (`PROSE-UI-204`).
    pub fn drop_text(&mut self, x: f32, y: f32, text: String) -> bool {
        self.feed(&ProseInput::Drop { x, y, text })
    }

    /// Another document of the same type: the state starts over, and the
    /// first layout of the new one arrives from the Logic thread.
    pub fn set_document(&mut self, document_id: DocumentId) {
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
fn download(filename: &str, media_type: &ambre_bus::MediaType, data: &[u8]) {
    use wasm_bindgen::JsCast;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(data).buffer());
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
        anchor.set_download(filename);
        anchor.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_prose::view::ruler::RulerHandle;
    use wasm_bindgen_test::wasm_bindgen_test;

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
        let (to_logic, _queue) = ambre_bus::conduit::local(4);
        let (_to_ui, from_logic) = ambre_bus::conduit::local(4);
        let pipe = TextInput::create(&document, &cell);
        let mut view = ProseView::new(
            DocumentId::new(),
            to_logic,
            from_logic,
            800.0,
            600.0,
            pipe.clone(),
            ThemeControl::new(Theme::Dark),
            RwSignal::new(None),
            ProseSignals::new(),
            BusyPublisher::new(),
            NoticePublisher::new(),
        );
        if let Some(body) = document.body() {
            let _ = body.focus();
        }
        let _ = view.handle_input(&InputEvent::PointerDown {
            x: 300.0,
            y: 200.0,
            button: 0,
            shift: false,
            ctrl: false,
        });
        let focused = document.active_element().expect("a focused element");
        assert!(
            focused.is_same_node(Some(pipe.element())),
            "{}",
            focused.tag_name()
        );
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
        let (to_logic, queue) = ambre_bus::conduit::local(1);
        let (_to_ui, from_logic) = ambre_bus::conduit::local(4);
        let document_id = DocumentId::new();
        let mut view = ProseView::new(
            document_id,
            to_logic,
            from_logic,
            800.0,
            600.0,
            TextInput::create(&document, &cell),
            ThemeControl::new(Theme::Dark),
            RwSignal::new(None),
            ProseSignals::new(),
            BusyPublisher::new(),
            NoticePublisher::new(),
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
            delta: ambre_canvas::WheelDelta::pixels(0.0, 40.0),
            ctrl_key: false,
        };
        view.dirty = false;
        assert!(view.handle_input(&wheel), "the refusal consumes the event");
        assert!(!view.dirty, "the busy page scrolled");
        queue.try_iter().for_each(drop);
        assert!(matches!(view.outbox.flush(), SendOutcome::Sent));
        assert!(view.handle_input(&wheel));
        assert!(view.dirty, "the drained page takes input again");
    }

    /// The ruler moves with the canvas and keeps its scale, at any zoom and any
    /// column offset (`PROSE-UI-065`). Each handle lands at the canvas
    /// position plus the canvas's own offset on the page.
    #[wasm_bindgen_test]
    fn the_ruler_in_css_pixels_follows_the_canvas_offset() {
        let origin = |x: f32, y: f32| (x + 40.0, y + 64.0);
        for (zoom, column_left) in [(1.0, 80.0), (2.0, 15.0), (0.5, 300.0)] {
            let canvas = RulerState {
                visible: true,
                top_px: 0.0,
                page_left_px: column_left,
                content_width_px: 451.0 * zoom,
                px_per_pt: 4.0 / 3.0 * zoom,
                indent_first: 18.0,
                indent_left: 36.0,
                indent_right: 12.0,
            };
            let css = ruler_in_css(canvas, origin);
            assert_eq!(css.top_px, 64.0);
            assert_eq!(css.content_width_px, canvas.content_width_px);
            for handle in [
                RulerHandle::IndentFirst,
                RulerHandle::IndentLeft,
                RulerHandle::IndentRight,
            ] {
                let moved = css.handle_x(handle) - canvas.handle_x(handle);
                assert!(
                    (moved - 40.0).abs() < 1e-3,
                    "zoom {zoom}: {handle:?} moved {moved}"
                );
            }
        }
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
        let target: web_sys::HtmlElement = slot.clone().dyn_into().expect("an element");
        mount_to(target, move || view! { <ProseFigures figures=figures /> }).forget();

        let (to_logic, _queue) = ambre_bus::conduit::local(4);
        let (_to_ui, from_logic) = ambre_bus::conduit::local(4);
        let mut view = ProseView::new(
            DocumentId::new(),
            to_logic,
            from_logic,
            800.0,
            600.0,
            TextInput::create(&document, &cell),
            ThemeControl::new(Theme::Dark),
            figures,
            ProseSignals::new(),
            BusyPublisher::new(),
            NoticePublisher::new(),
        );
        // The count as the page state holds it once the Logic thread's event
        // is applied. Decoding that event is Prose's own, and tested there.
        view.state.word_count = WordCount {
            words: 12,
            characters: 60,
            characters_with_spaces: 71,
            paragraphs: 1,
            pages: 2,
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
}
