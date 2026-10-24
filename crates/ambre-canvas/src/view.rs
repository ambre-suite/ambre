// ambre-canvas/src/view.rs
// ============================================================================
// Per-frame view value types: the GPU-agnostic vocabulary the frame driver and
// the app views share. `FrameStats` covers driver timings, `RepaintPlan` what to
// repaint, `InputEvent` a DOM event reduced to a flat enum.
//
// The frame DRIVER (`FrameHost`) and the closed `AppView` registry that names
// the concrete per-app view (`ambre_trame::ui::TrameView`) live in the launcher
// (`ambre-app`). A closed enum naming an app view cannot sit upstream here
// without a dependency cycle (ambre-spec-architecture.md). This module only
// defines the shared value types both sides exchange. App views consume them
// directly, through inherent methods, so the types stay GPU-agnostic at the type
// level. They only name `WgpuSurface` / `TextCache`, which compile on every
// target, and app crates can unit-test their view logic without a browser.
// ============================================================================

use ambre_core::{PageMillis, ViewPoint};
use ambre_draw::{Color, Rect};

use crate::{ContentRepaint, OverlayRepaint, WheelDelta};

/// Driver-measured per-frame timings, handed to the view so it can render a
/// debug HUD. All values are spans of the page clock from the *previous* frame.
#[derive(Default, Clone, Copy)]
pub struct FrameStats {
    pub fps: f32,
    pub frame_ms: PageMillis,
    pub drain_ms: PageMillis,
    pub draw_ms: PageMillis,
    pub gpu_ms: PageMillis,
}

/// A full per-frame performance snapshot, published from the canvas view to the
/// DOM status bar. The perf HUD migrated out of the wgpu overlay: there is no
/// point shaping debug glyphs on the GPU every frame. It carries the driver
/// timings ([`FrameStats`]) plus the view's own sub-phase breakdown and glyph
/// cache misses. The DOM panel shows the same detail the old overlay did.
#[derive(Default, Clone, Copy)]
pub struct PerfSnapshot {
    pub fps: f32,
    pub frame_ms: PageMillis,
    pub drain_ms: PageMillis,
    pub draw_ms: PageMillis,
    pub gpu_ms: PageMillis,
    pub bg_ms: PageMillis,
    pub cells_ms: PageMillis,
    pub hdrs_ms: PageMillis,
    pub cells_drawn: u32,
    pub input_lat_ms: PageMillis,
    pub glyph_misses: u32,
}

/// What the view wants repainted this frame.
///
/// `content` is the persistent layer (offscreen, supports scroll-strip reuse);
/// `overlay` is the transient swapchain layer. The driver translates this into
/// the `QuadRenderer::flush_dirty_rect` call. The scroll shift is in the pixel
/// space `P`: view pixels from a view, device pixels for the renderer.
pub struct RepaintPlan<P = ambre_core::ViewPx> {
    pub content: ContentRepaint<P>,
    pub content_strip: Option<Rect>,
    pub overlay: OverlayRepaint,
    /// The colour a full rebuild clears to, read from the view's palette. The
    /// renderer holds no colour of its own (`CANVAS-060`).
    pub backdrop: Color,
}

/// The button of a pointer press or release, as the DOM `button` field
/// numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    /// The main button: the left one, a touch or a pen contact.
    Primary,
    /// The middle button, or the wheel pressed.
    Auxiliary,
    /// The right button, which opens a context menu.
    Secondary,
    /// Any other: back, forward, or none.
    Other,
}

impl PointerButton {
    /// The button the DOM `button` field `button` names.
    pub const fn from_dom(button: i16) -> Self {
        match button {
            0 => Self::Primary,
            1 => Self::Auxiliary,
            2 => Self::Secondary,
            _ => Self::Other,
        }
    }
}

/// A raw input event forwarded from the DOM by the host's event listeners.
#[derive(Clone)]
pub enum InputEvent {
    Key {
        key: String,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    },
    /// A key released. A view that arms a mode on a held key disarms it here.
    /// A flag raised by a press and lowered by nothing outlives the gesture
    /// (`TOILE-UI-256`).
    KeyUp {
        key: String,
    },
    /// `shift` / `ctrl` carry the modifier state at press time. The grid can
    /// then distinguish a plain click (single select), Shift+click (extend the
    /// range) and Ctrl+click (add a disjoint range). On macOS the Cmd key is
    /// folded into `ctrl` by the host so the same disjoint-select gesture works.
    PointerDown {
        at: ViewPoint,
        button: PointerButton,
        shift: bool,
        ctrl: bool,
    },
    PointerMove {
        at: ViewPoint,
    },
    PointerUp {
        at: ViewPoint,
        button: PointerButton,
        /// Modifier state at *release* time (Cmd folded into ctrl). Autofill
        /// reads Ctrl here, not at grab time, so toggling it mid-drag counts.
        ctrl: bool,
    },
    /// Native double-click (respects the platform's double-click speed). Used to
    /// enter cell edit mode.
    DoubleClick {
        at: ViewPoint,
    },
    /// Wheel / two-finger touchpad scroll. The delta keeps the unit the
    /// browser gave it: CSS pixels (`CANVAS-032`), lines or pages. The view
    /// reads it through [`WheelDelta::to_px`] (`KINETIC-015`).
    Wheel {
        delta: WheelDelta,
        ctrl_key: bool,
    },

    /// Committed text from the transparent `<textarea>` pipe: latin typing,
    /// autocomplete or a plaintext paste. Insert it at the caret. The canvas
    /// owns the visual caret/selection; the textarea renders nothing.
    Text {
        text: String,
    },
    /// IME composition in progress (CJK/Korean/Vietnamese). `text` is the full
    /// preview string for this composition pass; it replaces the previous
    /// preview and is rendered with a composition style until committed.
    CompositionUpdate {
        text: String,
    },
    /// IME composition committed: the final `text` replaces the preview and is
    /// inserted into the model.
    CompositionEnd {
        text: String,
    },
}

#[cfg(test)]
mod tests {
    use super::PointerButton;

    /// The DOM numbers its buttons 0, 1 and 2; back, forward and the `-1`
    /// of a move with no button read as another.
    #[test]
    fn the_dom_button_number_reads_as_its_button() {
        assert_eq!(PointerButton::from_dom(0), PointerButton::Primary);
        assert_eq!(PointerButton::from_dom(1), PointerButton::Auxiliary);
        assert_eq!(PointerButton::from_dom(2), PointerButton::Secondary);
        for other in [-1, 3, 4] {
            assert_eq!(PointerButton::from_dom(other), PointerButton::Other);
        }
    }
}
