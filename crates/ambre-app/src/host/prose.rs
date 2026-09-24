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
// what they ask for once a frame.
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
pub use ambre_prose::chrome::ProseAnnotations;
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
    annotations: ProseAnnotations,
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
        annotations: ProseAnnotations,
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
                // The text moved from under the pointer.
                self.state.hovered = None;
                self.publish_hover();
                self.dirty = true;
                true
            }
            _ => {
                if let InputEvent::PointerMove { x, y } = ev
                    && self.state.hover(*x as f32, *y as f32)
                {
                    self.publish_hover();
                }
                let Some(input) = self.translate(ev) else {
                    return false;
                };
                let (outbox, document_id) = (&mut self.outbox, self.document_id);
                let mut send = |command| send_edit(outbox, document_id, command);
                let consumed = self.state.handle_input(&input, &mut send);
                self.busy_pub.publish(self.outbox.is_busy());
                self.dirty |= consumed;
                consumed
            }
        }
    }

    /// The input a canvas event stands for, or `None` for one the view leaves
    /// to the host. The composition lands with the "Toolbar, slash menu and
    /// composition" Story (`ambre-roadmap.md` E5).
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
            InputEvent::PointerDown {
                x,
                y,
                button: 0,
                shift,
                ..
            } => ProseInput::Press {
                x: *x as f32,
                y: *y as f32,
                shift: *shift,
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

    pub fn clipboard_write(&mut self, _cut: bool) -> ambre_bus::CopyOutcome {
        ambre_bus::CopyOutcome::Nothing
    }

    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        None
    }

    pub fn paste_clipboard(&mut self, _blob: ambre_bus::ClipboardBlob, _own_copy: bool) {}

    /// Another document of the same type: the state starts over, and the
    /// first layout of the new one arrives from the Logic thread.
    pub fn set_document(&mut self, document_id: DocumentId) {
        self.document_id = document_id;
        self.state = ProseUiState::default();
        self.state.resize(self.size.0, self.size.1);
        self.shown = None;
        self.publish_hover();
        self.dirty = true;
    }
}
