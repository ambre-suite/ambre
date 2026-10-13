// ambre-canvas/src/density.rs
// ============================================================================
// The display density, read in one place (`CANVAS-031`).
//
// A view lays out in logical pixels, the CSS pixels a layout is written
// against. The GPU fills physical pixels. The factor between the two is read
// here and nowhere else, and it never leaves this crate. A view is handed the
// logical size alone (`CANVAS-044`), so it has nothing to scale by.
// ============================================================================

use crate::render::ContentRepaint;
use crate::view::RepaintPlan;

/// The drawing buffer, in the two units of `ambre-spec-high-density.md`.
///
/// Only the logical size is public. The physical size and the density are
/// read by the renderer and the GPU bootstrap of this crate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSize {
    logical: (u32, u32),
    physical: (u32, u32),
    density: f64,
}

impl SurfaceSize {
    /// The backing store a browser gives a canvas nobody sized: 300 x 150.
    /// It stands in for a canvas that has no box yet.
    pub const UNSIZED: SurfaceSize = SurfaceSize {
        logical: (300, 150),
        physical: (300, 150),
        density: 1.0,
    };

    /// A CSS box of `width` x `height` at `density`, or `None` when the box
    /// has no area. A density that is not a positive number reads as 1.
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub(crate) fn new(width: u32, height: u32, density: f64) -> Option<Self> {
        let density = if density.is_finite() && density > 0.0 {
            density
        } else {
            1.0
        };
        let physical = (
            (f64::from(width) * density).round() as u32,
            (f64::from(height) * density).round() as u32,
        );
        if physical.0 == 0 || physical.1 == 0 {
            return None;
        }
        Some(Self {
            logical: (width, height),
            physical,
            density,
        })
    }

    /// The size a view lays out against, in CSS pixels.
    pub fn logical(&self) -> (u32, u32) {
        self.logical
    }

    /// The size of the drawing buffer, in device pixels.
    pub(crate) fn physical(&self) -> (u32, u32) {
        self.physical
    }

    /// Device pixels per CSS pixel.
    pub(crate) fn density(&self) -> f32 {
        self.density as f32
    }

    /// Turn the view's scroll shift into device pixels.
    ///
    /// A view shifts its content by a whole number of logical pixels. At a
    /// fractional density that shift can land between two device pixels, and
    /// the copy cannot. The plan then repaints in full rather than tear.
    pub fn shift_in_physical(&self, plan: &mut RepaintPlan) {
        let ContentRepaint::Scroll { dx_px, dy_px } = plan.content else {
            return;
        };
        let whole = |v: i32| {
            let p = f64::from(v) * self.density;
            let r = p.round();
            ((p - r).abs() < 1e-6).then_some(r as i32)
        };
        match (whole(dx_px), whole(dy_px)) {
            (Some(dx_px), Some(dy_px)) => plan.content = ContentRepaint::Scroll { dx_px, dy_px },
            _ => {
                plan.content = ContentRepaint::Full;
                plan.content_strip = None;
            }
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
    canvas.set_width(size.physical.0);
    canvas.set_height(size.physical.1);
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
    use crate::render::OverlayRepaint;

    fn scroll(dx_px: i32, dy_px: i32) -> RepaintPlan {
        RepaintPlan {
            content: ContentRepaint::Scroll { dx_px, dy_px },
            content_strip: Some(ambre_draw::Rect::new(0.0, 0.0, 10.0, 10.0)),
            overlay: OverlayRepaint::Reuse,
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
        assert_eq!(two.physical(), (1600, 1200));
    }

    /// A density the browser could not report reads as 1, never as a buffer
    /// of no size.
    #[test]
    fn an_unusable_density_reads_as_one() {
        for density in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            let size = SurfaceSize::new(300, 150, density).unwrap();
            assert_eq!(size.physical(), (300, 150));
        }
        assert!(SurfaceSize::new(0, 150, 2.0).is_none());
    }

    /// A shift is copied in device pixels, so it is scaled with the buffer.
    #[test]
    fn a_scroll_shift_is_copied_in_device_pixels() {
        let mut plan = scroll(0, -24);
        SurfaceSize::new(800, 600, 2.0)
            .unwrap()
            .shift_in_physical(&mut plan);
        assert!(matches!(
            plan.content,
            ContentRepaint::Scroll {
                dx_px: 0,
                dy_px: -48
            }
        ));
        assert!(plan.content_strip.is_some());
    }

    /// A shift that lands between two device pixels repaints in full. A copy
    /// rounded to a whole pixel would leave the strip a half pixel off.
    #[test]
    fn a_shift_between_two_device_pixels_repaints_in_full() {
        let mut plan = scroll(0, -25);
        SurfaceSize::new(800, 600, 1.25)
            .unwrap()
            .shift_in_physical(&mut plan);
        assert!(matches!(plan.content, ContentRepaint::Full));
        assert!(plan.content_strip.is_none());
    }
}
