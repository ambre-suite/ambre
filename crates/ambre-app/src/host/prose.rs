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
// Input beyond scrolling lands with the "Input and cursor" Story
// (`ambre-roadmap.md` E5).
// ============================================================================

use leptos::prelude::*;
use parley::FontContext;

use ambre_bus::app::prose::WordCount;
use ambre_bus::{AppEvent, DocumentId, LogicCommand, UiEvent};
use ambre_canvas::theme::{Theme, ThemeControl};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache, WgpuSurface,
};
use ambre_draw::{DrawSurface, Rect};
use ambre_prose::ui::{ProseUiState, send_command, status_figures};
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

/// One open Prose document on the canvas.
pub struct ProseView {
    document_id: DocumentId,
    to_logic: flume::Sender<LogicCommand>,
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
    ) -> Self {
        let theme = theme_ctl.get();
        let size = (width as f32, height as f32);
        let mut state = ProseUiState::default();
        state.resize(size.0, size.1);
        Self {
            document_id,
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
        if self.shown != Some(self.state.word_count) {
            self.shown = Some(self.state.word_count);
            self.figures
                .set(Some(status_figures(&self.state.word_count)));
        }
    }

    pub fn plan_repaint(&mut self, now_ms: f64, _hud_tick: bool) -> RepaintPlan {
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
        _font_cx: &mut FontContext,
        _text_cache: &mut TextCache,
        _strip: Option<Rect>,
    ) {
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
        match ev {
            InputEvent::Wheel { dy, delta_mode, .. } => {
                let px = match delta_mode {
                    1 => dy * LINE_PX,
                    2 => dy * f64::from(self.size.1),
                    _ => *dy,
                };
                self.state.scroll_by(px as f32);
                self.dirty = true;
                true
            }
            _ => false,
        }
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
        self.dirty = true;
    }
}
