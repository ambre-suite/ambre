// ambre-app/src/host/murmure.rs
// The canvas view of a Murmure page. The message list paints into this cell
// (the Murmure message list spec), and until it lands the view paints the
// backdrop alone. It opens no document: a Murmure page opens channels, and
// the channel the sidebar selects binds what it shows.

use ambre_bus::{DocumentId, UiEvent};
use ambre_canvas::theme::{Theme, ThemeControl};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache, WgpuSurface,
};
use ambre_draw::{DrawSurface, Rect};
use parley::FontContext;

/// The canvas view of a Murmure page.
pub struct MurmureView {
    from_logic: ambre_bus::conduit::Receiver<UiEvent>,
    theme_ctl: ThemeControl,
    theme: Theme,
    size: (f32, f32),
    dirty: bool,
}

impl MurmureView {
    pub fn new(
        from_logic: ambre_bus::conduit::Receiver<UiEvent>,
        width: f64,
        height: f64,
        theme_ctl: ThemeControl,
    ) -> Self {
        let theme = theme_ctl.get();
        Self {
            from_logic,
            theme_ctl,
            theme,
            size: (width as f32, height as f32),
            dirty: true,
        }
    }

    /// Drain the inbound queue, so a full queue never stalls the Logic thread.
    /// The message list reads it once it lands.
    pub fn update(&mut self) {
        while self.from_logic.try_recv().is_ok() {}
    }

    pub fn plan_repaint(&mut self, _now_ms: f64, _hud_tick: bool) -> RepaintPlan {
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
            overlay: OverlayRepaint::Reuse,
            backdrop: self.theme.palette().content_bg,
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
        surface.fill_rect(backdrop, self.theme.palette().content_bg);
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
        self.dirty = true;
    }

    /// Nothing on the canvas takes input before the message list lands.
    pub fn handle_input(&mut self, _ev: &InputEvent) -> bool {
        false
    }

    /// A Murmure page binds no document, so a route swap changes nothing.
    pub fn set_document(&mut self, _document_id: DocumentId) {}
}
