// ambre-canvas/src/density.rs
// ============================================================================
// The display density, read in one place (`CANVAS-031`).
//
// A view lays out in logical pixels, the CSS pixels a layout is written
// against. The GPU fills physical pixels. The factor between the two is read
// here and nowhere else, and it never leaves this crate. A view is handed the
// logical size alone (`CANVAS-044`), so it has nothing to scale by.
// ============================================================================

pub use ambre_core::PixelSize;
use ambre_core::{CssPx, ViewPx};

use crate::render::ContentRepaint;
use crate::view::RepaintPlan;

/// A whole number of device pixels: a texel of the drawing buffer.
///
/// A shift of the buffer is signed, so the type is too. Nothing turns a CSS or
/// a view pixel into one but the density of [`SurfaceSize`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DevicePx(i32);

impl DevicePx {
    /// No shift.
    pub const ZERO: Self = Self(0);

    /// The length of `value` device pixels.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// The number of device pixels, for a foreign API that takes the integer.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }

    /// The length without its sign, in the unsigned integer a texture
    /// coordinate takes.
    #[must_use]
    pub const fn unsigned_abs(self) -> u32 {
        self.0.unsigned_abs()
    }
}

impl std::ops::Neg for DevicePx {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// Device pixels per CSS pixel: the display density.
///
/// A density is a positive, finite number. A value the browser gives that is
/// not one reads as 1, so no buffer is sized by a zero or a NaN.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub(crate) struct DevicePixelRatio(f64);

impl DevicePixelRatio {
    /// One device pixel per CSS pixel.
    pub(crate) const ONE: Self = Self(1.0);

    /// The density `ratio`, or `None` when it is not a positive finite number.
    #[must_use]
    pub(crate) fn new(ratio: f64) -> Option<Self> {
        (ratio.is_finite() && ratio > 0.0).then_some(Self(ratio))
    }

    /// The ratio, for the arithmetic that scales a length by it.
    #[must_use]
    pub(crate) const fn get(self) -> f64 {
        self.0
    }
}

/// The CSS box of a canvas, the size a view lays out against.
pub type CssSize = PixelSize<CssPx>;

/// The drawing buffer, in device pixels.
pub type DeviceSize = PixelSize<DevicePx>;

/// The drawing buffer, in the two units of `ambre-spec-high-density.md`.
///
/// Only the logical size is public. The physical size and the density are
/// read by the renderer and the GPU bootstrap of this crate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSize {
    logical: CssSize,
    physical: DeviceSize,
    density: DevicePixelRatio,
}

impl SurfaceSize {
    /// The backing store a browser gives a canvas nobody sized: 300 x 150.
    /// It stands in for a canvas that has no box yet.
    pub const UNSIZED: SurfaceSize = SurfaceSize {
        logical: CssSize::new(300, 150),
        physical: DeviceSize::new(300, 150),
        density: DevicePixelRatio::ONE,
    };

    /// A CSS box of `width` x `height` at `density`, or `None` when the box
    /// has no area. A density that is not a positive number reads as 1.
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub(crate) fn new(width: u32, height: u32, density: f64) -> Option<Self> {
        let density = DevicePixelRatio::new(density).unwrap_or(DevicePixelRatio::ONE);
        let physical = DeviceSize::new(
            (f64::from(width) * density.get()).round() as u32,
            (f64::from(height) * density.get()).round() as u32,
        );
        if physical.width == 0 || physical.height == 0 {
            return None;
        }
        Some(Self {
            logical: CssSize::new(width, height),
            physical,
            density,
        })
    }

    /// The size a view lays out against, in CSS pixels.
    pub fn logical(&self) -> CssSize {
        self.logical
    }

    /// The size of the drawing buffer, in device pixels.
    pub(crate) fn physical(&self) -> DeviceSize {
        self.physical
    }

    /// Device pixels per CSS pixel.
    pub(crate) fn density(&self) -> DevicePixelRatio {
        self.density
    }

    /// The view's plan on the drawing buffer: its scroll shift in device
    /// pixels.
    ///
    /// A view shifts its content by a whole number of view pixels. At a
    /// fractional density that shift can land between two device pixels, and
    /// the copy cannot. The plan then repaints in full rather than tear.
    pub fn on_device(&self, plan: RepaintPlan) -> RepaintPlan<DevicePx> {
        let whole = |v: ViewPx| {
            let p = f64::from(v.get()) * self.density.get();
            let r = p.round();
            ((p - r).abs() < 1e-6).then_some(DevicePx::new(r as i32))
        };
        let (content, content_strip) = match plan.content {
            ContentRepaint::Full => (ContentRepaint::Full, plan.content_strip),
            ContentRepaint::Reuse => (ContentRepaint::Reuse, plan.content_strip),
            ContentRepaint::Scroll { dx, dy } => match (whole(dx), whole(dy)) {
                (Some(dx), Some(dy)) => (ContentRepaint::Scroll { dx, dy }, plan.content_strip),
                _ => (ContentRepaint::Full, None),
            },
        };
        RepaintPlan {
            content,
            content_strip,
            overlay: plan.overlay,
            backdrop: plan.backdrop,
        }
    }
}

/// Size the canvas backing store to its CSS box times the density, and answer
/// both sizes. `None` when the box has no area, detached or `display: none`.
#[cfg(target_arch = "wasm32")]
pub fn size_backing_store(canvas: &web_sys::HtmlCanvasElement) -> Option<SurfaceSize> {
    let size = SurfaceSize::new(
        canvas.client_width().max(0) as u32,
        canvas.client_height().max(0) as u32,
        read(),
    )?;
    canvas.set_width(size.physical.width);
    canvas.set_height(size.physical.height);
    Some(size)
}

/// The media query that stops matching when the density moves.
///
/// A query fires only when its own threshold is crossed, so the watch is
/// re-armed on a fresh query after each change (`CANVAS-045`).
#[cfg(target_arch = "wasm32")]
pub fn density_query() -> String {
    format!("(resolution: {}dppx)", read())
}

/// A watch on the density, re-armed on a fresh query each time it fires.
///
/// A `ResizeObserver` stays silent when only the density moves, so the host
/// resizes from here (`CANVAS-035`). The query that fired no longer matches,
/// so each change arms the next one (`CANVAS-045`).
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct DensityWatch {
    /// The query armed now, and its listener. `None` once the browser refused one.
    armed: std::rc::Rc<std::cell::RefCell<Option<Armed>>>,
}

/// One armed query and the listener on it, kept so the listener can be removed.
#[cfg(target_arch = "wasm32")]
struct Armed {
    query: web_sys::MediaQueryList,
    listener: wasm_bindgen::JsValue,
}

#[cfg(target_arch = "wasm32")]
impl DensityWatch {
    /// Arm the watch on the current density. `on_change` runs at each move.
    pub fn arm(on_change: std::rc::Rc<dyn Fn()>) -> Self {
        let watch = Self {
            armed: std::rc::Rc::default(),
        };
        watch.rearm(on_change);
        watch
    }

    /// The query armed now, so a test can raise its `change`.
    pub fn armed(&self) -> Option<web_sys::MediaQueryList> {
        self.armed.borrow().as_ref().map(|a| a.query.clone())
    }

    /// Drop the listener on the query that fired, then arm one on the new
    /// density. The old listener runs once only, so it must not stay attached.
    fn rearm(&self, on_change: std::rc::Rc<dyn Fn()>) {
        use wasm_bindgen::JsCast;
        if let Some(old) = self.armed.borrow_mut().take() {
            let _ = old
                .query
                .remove_event_listener_with_callback("change", old.listener.unchecked_ref());
        }
        let Some(query) =
            web_sys::window().and_then(|w| w.match_media(&density_query()).ok().flatten())
        else {
            return;
        };
        let watch = self.clone();
        let listener = wasm_bindgen::closure::Closure::once_into_js(move |_: web_sys::Event| {
            on_change();
            watch.rearm(on_change);
        });
        let _ = query.add_event_listener_with_callback("change", listener.unchecked_ref());
        *self.armed.borrow_mut() = Some(Armed { query, listener });
    }
}

/// The browser's density. The one read of it in the workspace (`CANVAS-031`).
#[cfg(target_arch = "wasm32")]
fn read() -> f64 {
    web_sys::window().map_or(1.0, |w| w.device_pixel_ratio())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::LayerRepaint;

    fn scroll(dx: f32, dy: f32) -> RepaintPlan {
        RepaintPlan {
            content: ContentRepaint::Scroll {
                dx: ViewPx::new(dx),
                dy: ViewPx::new(dy),
            },
            content_strip: Some(ambre_draw::Rect::new(0.0, 0.0, 10.0, 10.0)),
            overlay: LayerRepaint::Reuse,
            backdrop: ambre_draw::Color::WHITE,
        }
    }

    /// The same CSS box on a denser screen is the same layout (`CANVAS-046`).
    /// Only the buffer behind it grows.
    #[test]
    fn a_density_change_keeps_the_logical_size() {
        let one = SurfaceSize::new(800, 600, 1.0).unwrap();
        let two = SurfaceSize::new(800, 600, 2.0).unwrap();
        assert_eq!(one.logical(), two.logical());
        assert_eq!(two.physical(), DeviceSize::new(1600, 1200));
    }

    /// A density the browser could not report reads as 1, never as a buffer
    /// of no size.
    #[test]
    fn an_unusable_density_reads_as_one() {
        for density in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(DevicePixelRatio::new(density), None);
            let size = SurfaceSize::new(300, 150, density).unwrap();
            assert_eq!(size.density(), DevicePixelRatio::ONE);
            assert_eq!(size.physical(), DeviceSize::new(300, 150));
        }
        assert!(SurfaceSize::new(0, 150, 2.0).is_none());
    }

    /// A shift is copied in device pixels, so it is scaled with the buffer.
    #[test]
    fn a_scroll_shift_is_copied_in_device_pixels() {
        let plan = SurfaceSize::new(800, 600, 2.0)
            .unwrap()
            .on_device(scroll(0.0, -24.0));
        assert!(matches!(
            plan.content,
            ContentRepaint::Scroll { dx, dy } if dx == DevicePx::ZERO && dy == DevicePx::new(-48)
        ));
        assert!(plan.content_strip.is_some());
    }

    /// A shift that lands between two device pixels repaints in full. A copy
    /// rounded to a whole pixel would leave the strip a half pixel off.
    #[test]
    fn a_shift_between_two_device_pixels_repaints_in_full() {
        let plan = SurfaceSize::new(800, 600, 1.25)
            .unwrap()
            .on_device(scroll(0.0, -25.0));
        assert!(matches!(plan.content, ContentRepaint::Full));
        assert!(plan.content_strip.is_none());
    }
}
