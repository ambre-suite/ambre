// ambre-canvas/src/view.rs
// ============================================================================
// Per-frame view value types -- the GPU-agnostic vocabulary the frame driver
// and the app views share: `FrameStats` (driver timings), `RepaintPlan` (what
// to repaint), `InputEvent` (a DOM event reduced to a flat enum).
//
// The frame DRIVER (`FrameHost`) and the closed `AppView` registry that names
// the concrete per-app view (`ambre_trame::ui::TrameView`) live in the launcher
// (`ambre-app`): a closed enum naming an app view cannot sit upstream here
// without a dependency cycle (ambre-spec-architecture.md 6). This module only
// defines the shared value types both sides exchange; app views consume them
// directly (inherent methods), so the types stay GPU-agnostic at the type level
// (they only name `WgpuSurface`/`TextCache`, which compile on every target) and
// app crates can unit-test their view logic without a browser.
// ============================================================================

use ambre_draw::Rect;

use crate::{ContentRepaint, OverlayRepaint};

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
    /// Wheel / two-finger touchpad scroll. `dx`/`dy` are already scaled to
    /// physical pixels (CSS px × dpr). `delta_mode` is the DOM `WheelEvent`
    /// unit: 0 = pixel (touchpad, modern mice), 1 = line, 2 = page; the view
    /// normalises lines/pages to pixels before scrolling.
    Wheel { dx: f64, dy: f64, delta_mode: u32, ctrl_key: bool },

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

