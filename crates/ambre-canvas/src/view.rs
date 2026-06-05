// ambre-canvas/src/view.rs
// ============================================================================
// DocView — the per-application UI contract driven by the generic FrameHost.
//
// The FrameHost (host.rs) owns the GPU, the rAF loop, channel-drain timing and
// the swapchain present. It knows nothing about spreadsheets, documents or
// canvases. Everything app-specific — what changed, what to paint, how to react
// to input — is delegated to a `DocView` implementation living in the app crate
// (e.g. `ambre_trame::ui::TrameView`).
//
// This module is GPU-agnostic at the type level (it only names `WgpuSurface`,
// which compiles on every target) so app crates can unit-test their view logic
// without a browser.
// ============================================================================

use parley::FontContext;

use ambre_draw::Rect;

use crate::{ContentRepaint, OverlayRepaint, TextCache, WgpuSurface};

/// Driver-measured per-frame timings, handed to the view so it can render a
/// debug HUD. All values are wall-clock milliseconds from the *previous* frame.
#[derive(Default, Clone, Copy)]
pub struct FrameStats {
    pub fps:      f32,
    pub frame_ms: f32,
    pub drain_ms: f32,
    pub draw_ms:  f32,
    pub gpu_ms:   f32,
}

/// What the view wants repainted this frame.
///
/// `content` is the persistent layer (offscreen, supports scroll-strip reuse);
/// `overlay` is the transient swapchain layer. The driver translates this into
/// the `QuadRenderer::flush_dirty_rect` call.
pub struct RepaintPlan {
    pub content:       ContentRepaint,
    pub content_strip: Option<Rect>,
    pub overlay:       OverlayRepaint,
}

/// A raw input event forwarded from the DOM by the host's event listeners.
pub enum InputEvent {
    Key { key: String, ctrl: bool, alt: bool, shift: bool, meta: bool },
    PointerDown { x: f64, y: f64, button: i16 },
    PointerMove { x: f64, y: f64 },
    PointerUp { x: f64, y: f64, button: i16 },
    /// Native double-click (respects the platform's double-click speed). Used to
    /// enter cell edit mode.
    DoubleClick { x: f64, y: f64 },
    Wheel { dx: f64, dy: f64, ctrl_key: bool },

    /// Committed text from the transparent `<textarea>` pipe: latin typing,
    /// autocomplete or a plaintext paste. Insert it at the caret. The canvas
    /// owns the visual caret/selection; the textarea renders nothing.
    Text { text: String },
    /// IME composition in progress (CJK/Korean/Vietnamese). `text` is the full
    /// preview string for this composition pass; it replaces the previous
    /// preview and is rendered with a composition style until committed.
    CompositionUpdate { text: String },
    /// IME composition committed: the final `text` replaces the preview and is
    /// inserted into the model.
    CompositionEnd { text: String },
}

/// Per-application UI logic driven once per animation frame by [`FrameHost`].
///
/// Call order each frame: [`update`] → [`plan_repaint`] → (optionally)
/// [`paint_content`] / [`paint_overlay`]. [`resize`] and [`handle_input`] are
/// called out-of-band from the host's DOM listeners.
///
/// [`update`]: DocView::update
/// [`plan_repaint`]: DocView::plan_repaint
/// [`paint_content`]: DocView::paint_content
/// [`paint_overlay`]: DocView::paint_overlay
/// [`resize`]: DocView::resize
/// [`handle_input`]: DocView::handle_input
pub trait DocView {
    /// Pump inbound state: drain logic-thread messages, apply them, and set
    /// internal dirty flags. Called before painting, with no GPU access.
    fn update(&mut self);

    /// Decide what to repaint this frame and clear the dirty flags.
    ///
    /// `hud_tick` is true when the host's periodic HUD refresh (~2 Hz) fires;
    /// the view should redraw its overlay in that case even if nothing else
    /// changed.
    fn plan_repaint(&mut self, hud_tick: bool) -> RepaintPlan;

    /// Paint the persistent content layer. `strip` is `Some` for a scroll repaint
    /// (only that rect is redrawn over shifted pixels), `None` for full.
    fn paint_content(
        &mut self,
        surface:    &mut WgpuSurface<'_>,
        font_cx:    &mut FontContext,
        text_cache: &mut TextCache,
        strip:      Option<Rect>,
    );

    /// Paint the transient overlay layer. `stats` carries the host's timing
    /// numbers for the debug HUD.
    fn paint_overlay(
        &mut self,
        surface:    &mut WgpuSurface<'_>,
        font_cx:    &mut FontContext,
        text_cache: &mut TextCache,
        stats:      &FrameStats,
    );

    /// Canvas resized to `width` × `height` physical pixels.
    fn resize(&mut self, width: u32, height: u32);

    /// Handle an input event. Returns `true` if the host should call
    /// `preventDefault()` on the originating DOM event.
    fn handle_input(&mut self, ev: &InputEvent) -> bool;
}
