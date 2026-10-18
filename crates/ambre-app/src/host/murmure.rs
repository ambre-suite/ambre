// ambre-app/src/host/murmure.rs
// The canvas view of a Murmure message region. The message list paints into
// this cell (the Murmure message list spec), and until it lands the view paints
// the backdrop alone. It opens no document: a Murmure page opens channels, and
// the channel the sidebar selects binds what it shows.
//
// The channel is the content canvas. The thread is a canvas of its own in the
// panel, a cell the host binds by its id (see `cell`).

use ambre_bus::{DocumentId, UiEvent};
use ambre_canvas::theme::{Theme, ThemeControl};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, RepaintPlan, TextCache, WgpuSurface,
};
use ambre_draw::{DrawSurface, Point, Rect};
use parley::FontContext;

/// The id of the thread's canvas, in the panel. The shell names it, and the
/// host binds a cell to it while it is in the page.
pub const THREAD_CANVAS: &str = "murmure-thread-canvas";

/// The canvas view of a Murmure message region.
pub struct MurmureView {
    /// The events of the Logic thread. The content canvas holds the page's
    /// queue; a cell beside it holds none until the message list lands.
    from_logic: Option<ambre_bus::conduit::Receiver<UiEvent>>,
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
        from_logic: Option<ambre_bus::conduit::Receiver<UiEvent>>,
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
            hover: None,
            dirty: true,
        }
    }

    /// The view of another region of the page, such as the thread: the same
    /// theme, and no queue of its own.
    pub fn region(&self, width: f64, height: f64) -> Self {
        Self::new(None, width, height, self.theme_ctl.clone())
    }

    /// The size this region lays out in, in logical pixels.
    #[cfg(test)]
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Where the pointer last was over this region.
    #[cfg(test)]
    pub fn hover(&self) -> Option<Point> {
        self.hover
    }

    /// Drain the inbound queue, so a full queue never stalls the Logic thread.
    /// The message list reads it once it lands.
    pub fn update(&mut self) {
        if let Some(from_logic) = &self.from_logic {
            while from_logic.try_recv().is_ok() {}
        }
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

    /// The region follows the pointer. Nothing on it consumes an event before
    /// the message list lands.
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
