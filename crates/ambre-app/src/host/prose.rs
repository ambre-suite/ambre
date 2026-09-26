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
use ambre_bus::{AppEvent, DocumentId, LogicCommand, Outbox, SendOutcome, UiEvent};
use ambre_canvas::theme::{Theme, ThemeControl};
use ambre_canvas::{
    BusyPublisher, ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache,
    WgpuSurface,
};
use ambre_draw::{DrawSurface, ParleyBrush, Rect};
pub use ambre_prose::chrome::ProseSignals;
use ambre_prose::input::{ProseInput, ProseKey};
use ambre_prose::ui::{ProseUiState, send_command, send_edit, status_figures};
use ambre_prose::view::annotate::{HoveredSuggestion, PanelThread, ScreenRect};
use ambre_prose::view::paint::PagePalette;

/// The pixels one wheel line scrolls, for a device that reports lines.
const LINE_PX: f64 = 40.0;

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

/// From canvas pixels to CSS pixels of the window.
fn to_css(x: f32, y: f32) -> (f32, f32) {
    let window = web_sys::window();
    let dpr = window
        .as_ref()
        .map(|w| w.device_pixel_ratio())
        .filter(|dpr| *dpr > 0.0)
        .unwrap_or(1.0) as f32;
    let (left, top) = window
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".ambre-canvas").ok().flatten())
        .map_or((0.0, 0.0), |el| {
            let r = el.get_bounding_client_rect();
            (r.left() as f32, r.top() as f32)
        });
    (left + x / dpr, top + y / dpr)
}

fn rect_to_css(rect: ScreenRect) -> ScreenRect {
    let (x0, y0) = to_css(rect.x0, rect.y0);
    let (x1, y1) = to_css(rect.x1, rect.y1);
    ScreenRect { x0, y0, x1, y1 }
}

/// One open Prose document on the canvas.
pub struct ProseView {
    document_id: DocumentId,
    to_logic: flume::Sender<LogicCommand>,
    /// The edits' path out: a full channel holds them here (`BUS-011`).
    outbox: Outbox,
    busy_pub: BusyPublisher,
    from_logic: flume::Receiver<UiEvent>,
    state: ProseUiState,
    theme_ctl: ThemeControl,
    theme: Theme,
    size: (f32, f32),
    /// Something visible changed since the last content paint.
    dirty: bool,
    fatal: bool,
    /// The five figures as the status bar slot shows them.
    figures: RwSignal<Option<[String; 5]>>,
    shown: Option<WordCount>,
    annotations: ProseSignals,
    /// The layout context the draft is shaped with.
    layout_cx: LayoutContext<ParleyBrush>,
    /// The clock of the last frame, to count presses in place.
    now_ms: f64,
}

impl ProseView {
    pub fn new(
        document_id: DocumentId,
        to_logic: flume::Sender<LogicCommand>,
        from_logic: flume::Receiver<UiEvent>,
        width: f64,
        height: f64,
        theme_ctl: ThemeControl,
        figures: RwSignal<Option<[String; 5]>>,
        annotations: ProseSignals,
        busy_pub: BusyPublisher,
    ) -> Self {
        let theme = theme_ctl.get();
        let size = (width as f32, height as f32);
        let mut state = ProseUiState::default();
        state.resize(size.0, size.1);
        Self {
            document_id,
            outbox: Outbox::new(to_logic.clone()),
            busy_pub,
            to_logic,
            from_logic,
            state,
            theme_ctl,
            theme,
            size,
            dirty: true,
            fatal: false,
            figures,
            shown: None,
            annotations,
            layout_cx: LayoutContext::new(),
            now_ms: 0.0,
        }
    }

    fn signal_fatal(&mut self, message: &str) {
        if !self.fatal {
            self.fatal = true;
            ambre_canvas::signal_fatal(message);
        }
    }

    pub fn update(&mut self) {
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
                    self.state.apply_event(&event);
                    if let Some((filename, media_type, data)) = self.state.take_export() {
                        download(&filename, media_type, &data);
                    }
                    self.dirty = true;
                }
                Ok(UiEvent::FatalError { message }) => self.signal_fatal(&message),
                Ok(_) => {}
                Err(flume::TryRecvError::Empty) => break,
                Err(flume::TryRecvError::Disconnected) => {
                    self.signal_fatal("Logic thread disconnected");
                    break;
                }
            }
        }
        // An accepted or rejected suggestion takes its popover along.
        self.publish_hover();
        self.take_intent();
        self.take_format();
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
        let now_ms = js_sys::Date::now() as u64;
        if let Some(command) = self.state.take_intent(intent, now_ms) {
            send_edit(&mut self.outbox, self.document_id, command);
            self.busy_pub.publish(self.outbox.is_busy());
        }
        self.publish_hover();
        self.dirty = true;
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

    pub fn plan_repaint(&mut self, now_ms: f64, _hud_tick: bool) -> RepaintPlan {
        self.now_ms = now_ms;
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
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.dirty = true;
        }
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
        }
    }

    pub fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        _text_cache: &mut TextCache,
        _strip: Option<Rect>,
    ) {
        self.state.shape_draft(font_cx, &mut self.layout_cx);
        let backdrop = Rect::new(0.0, 0.0, f64::from(self.size.0), f64::from(self.size.1));
        surface.fill_rect(backdrop, self.theme.palette().content_pending);
        self.state.paint(surface, &page_palette(self.theme));
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
        // An edit waits in the outbox, so the page refuses input until it
        // drains (`BUS-016`). Consuming the event is what makes the refusal
        // visible, as on the board and the grid.
        if self.outbox.is_busy() {
            return true;
        }
        match ev {
            InputEvent::Wheel { dy, delta_mode, .. } => {
                let px = match delta_mode {
                    1 => dy * LINE_PX,
                    2 => dy * f64::from(self.size.1),
                    _ => *dy,
                };
                self.state.scroll_by(px as f32);
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
                let (outbox, document_id) = (&mut self.outbox, self.document_id);
                let mut send = |command| send_edit(outbox, document_id, command);
                let consumed = self.state.handle_input(&input, &mut send);
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
                let ctrl = *ctrl || *meta;
                ProseInput::Key {
                    key: ProseKey::from_name(key, ctrl)?,
                    shift: *shift,
                    ctrl,
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
                shift: *shift,
                // The canvas folds `Cmd` into `ctrl`.
                follow: *ctrl,
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
    pub fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
        if self.state.selection().is_none() {
            return ambre_bus::CopyOutcome::Nothing;
        }
        if self.feed(&ProseInput::Copy { cut }) {
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
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
        let input = self.state.paste_input(blob, own_copy);
        self.feed(&input);
    }

    /// Another document of the same type: the state starts over, and the
    /// first layout of the new one arrives from the Logic thread.
    pub fn set_document(&mut self, document_id: DocumentId) {
        self.document_id = document_id;
        self.state = ProseUiState::default();
        self.state.resize(self.size.0, self.size.1);
        self.shown = None;
        self.publish_hover();
        self.publish_toolbar();
        self.dirty = true;
    }
}

/// Hand a file the Logic thread wrote to the browser as a download, typed with
/// its media type (`PROSE-IO-056`). A browser that refuses is logged: the
/// document itself is untouched either way.
fn download(filename: &str, media_type: &str, data: &[u8]) {
    use wasm_bindgen::JsCast;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(data).buffer());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(media_type);
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
