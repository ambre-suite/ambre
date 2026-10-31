// ambre-canvas/src/density.rs
// ============================================================================
// The display density, read in one place (`CANVAS-031`).
//
// A view lays out in logical pixels, the CSS pixels a layout is written
// against. The GPU fills physical pixels. The factor between the two is read
// here and nowhere else, and it never leaves this crate. A view is handed the
// logical size alone (`CANVAS-044`), so it has nothing to scale by.
// ============================================================================

use ambre_core::{Css, Device, Px, Scale, Size, Texel, View, WholeCss};

use crate::render::ContentRepaint;
use crate::view::RepaintPlan;

/// The drawing buffer, in the two units of `ambre-spec-high-density.md`.
///
/// Only the logical size is public. The physical size and the density are
/// read by the renderer and the GPU bootstrap of this crate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSize {
    logical: Size<Px<WholeCss>>,
    physical: Size<Px<Texel>>,
    density: Scale<Css, Device>,
}

impl SurfaceSize {
    /// The backing store a browser gives a canvas nobody sized: 300 x 150.
    /// It stands in for a canvas that has no box yet.
    pub const UNSIZED: SurfaceSize = SurfaceSize {
        logical: Size::new(Px::new(300), Px::new(150)),
        physical: Size::new(Px::new(300), Px::new(150)),
        density: Scale::<Css, Device>::ONE,
    };

    /// The CSS box `logical` at `density`, or `None` when the drawing buffer
    /// has no area.
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub(crate) fn new(logical: Size<Px<WholeCss>>, density: Scale<Css, Device>) -> Option<Self> {
        let scale =
            |side: Px<WholeCss>| Px::new((f64::from(side.get()) * density.get()).round() as u32);
        let physical = Size::new(scale(logical.width), scale(logical.height));
        if physical.width == Px::ZERO || physical.height == Px::ZERO {
            return None;
        }
        Some(Self {
            logical,
            physical,
            density,
        })
    }

    /// The size a view lays out against, in CSS pixels.
    pub fn logical(&self) -> Size<Px<WholeCss>> {
        self.logical
    }

    /// The same size in view pixels, which a surface paints in. A view pixel
    /// is a CSS pixel in size (`CANVAS-032`).
    pub fn view(&self) -> Size<Px<View>> {
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (
            self.logical.width.get() as f32,
            self.logical.height.get() as f32,
        );
        Size::new(Px::new(width), Px::new(height))
    }

    /// The size of the drawing buffer, in device pixels.
    pub(crate) fn physical(&self) -> Size<Px<Texel>> {
        self.physical
    }

    /// Device pixels per CSS pixel.
    pub(crate) fn density(&self) -> Scale<Css, Device> {
        self.density
    }

    /// The view's plan on the drawing buffer: its scroll shift in device
    /// pixels.
    ///
    /// A view shifts its content by a whole number of view pixels. At a
    /// fractional density that shift can land between two device pixels, and
    /// the copy cannot. The plan then repaints in full rather than tear.
    pub fn on_device(&self, plan: RepaintPlan) -> RepaintPlan<Px<Device>> {
        let whole = |v: Px<View>| {
            let p = f64::from(v.get()) * self.density.get();
            let r = p.round();
            ((p - r).abs() < 1e-6).then_some(Px::<Device>::new(r as i32))
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
    let side = |client: i32| Px::new(client.max(0) as u32);
    let logical = Size::new(side(canvas.client_width()), side(canvas.client_height()));
    let size = SurfaceSize::new(logical, read())?;
    canvas.set_width(size.physical.width.get());
    canvas.set_height(size.physical.height.get());
    Some(size)
}

/// The media query that stops matching when the density moves.
///
/// A query fires only when its own threshold is crossed, so the watch is
/// re-armed on a fresh query after each change (`CANVAS-045`).
#[cfg(target_arch = "wasm32")]
pub fn density_query() -> String {
    format!("(resolution: {}dppx)", read().get())
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
/// A value that is not a positive finite number reads as 1, so no buffer is
/// sized by a zero or a NaN.
#[cfg(target_arch = "wasm32")]
fn read() -> Scale<Css, Device> {
    web_sys::window()
        .and_then(|w| Scale::new(w.device_pixel_ratio()))
        .unwrap_or(Scale::ONE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::LayerRepaint;

    fn scroll(dx: f32, dy: f32) -> RepaintPlan {
        RepaintPlan {
            content: ContentRepaint::Scroll {
                dx: Px::<View>::new(dx),
                dy: Px::<View>::new(dy),
            },
            content_strip: Some(ambre_draw::Rect::new(0.0, 0.0, 10.0, 10.0)),
            overlay: LayerRepaint::Reuse,
            backdrop: ambre_draw::Color::WHITE,
        }
    }

    /// The density a browser reports, read as `read` reads it: 1 when it is
    /// not a positive finite number.
    fn reported(ratio: f64) -> Scale<Css, Device> {
        Scale::new(ratio).unwrap_or(Scale::ONE)
    }

    /// A CSS box of `width` x `height` at the reported `density`.
    fn surface(width: u32, height: u32, density: f64) -> Option<SurfaceSize> {
        SurfaceSize::new(
            Size::new(Px::new(width), Px::new(height)),
            reported(density),
        )
    }

    /// The same CSS box on a denser screen is the same layout (`CANVAS-046`).
    /// Only the buffer behind it grows.
    #[test]
    fn a_density_change_keeps_the_logical_size() {
        let one = surface(800, 600, 1.0).unwrap();
        let two = surface(800, 600, 2.0).unwrap();
        assert_eq!(one.logical(), two.logical());
        assert_eq!(two.physical(), Size::new(Px::new(1600), Px::new(1200)));
    }

    /// A density the browser could not report reads as 1, never as a buffer
    /// of no size.
    #[test]
    fn an_unusable_density_reads_as_one() {
        for density in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(Scale::<Css, Device>::new(density), None);
            assert_eq!(reported(density), Scale::<Css, Device>::ONE);
            let size = surface(300, 150, density).unwrap();
            assert_eq!(size.density(), Scale::<Css, Device>::ONE);
            assert_eq!(size.physical(), Size::new(Px::new(300), Px::new(150)));
        }
        assert!(surface(0, 150, 2.0).is_none());
    }

    /// A density so small that the buffer rounds to no pixel is a box with
    /// no area, never a buffer of no size.
    #[test]
    fn a_buffer_that_rounds_to_nothing_has_no_size() {
        assert!(surface(1, 1, 0.25).is_none());
        assert!(surface(4, 4, 0.25).is_some());
    }

    /// A view lays out in CSS pixels, whatever the density.
    #[test]
    fn the_view_size_is_the_css_box() {
        let two = surface(800, 600, 2.0).unwrap();
        assert_eq!(
            two.view(),
            ambre_core::Size::new(Px::<View>::new(800.0), Px::<View>::new(600.0))
        );
        assert_eq!(surface(800, 600, 1.0).unwrap().view(), two.view());
    }

    /// A shift is copied in device pixels, so it is scaled with the buffer.
    #[test]
    fn a_scroll_shift_is_copied_in_device_pixels() {
        let plan = surface(800, 600, 2.0)
            .unwrap()
            .on_device(scroll(0.0, -24.0));
        assert!(matches!(
            plan.content,
            ContentRepaint::Scroll { dx, dy } if dx == Px::<Device>::ZERO && dy == Px::<Device>::new(-48)
        ));
        assert!(plan.content_strip.is_some());
    }

    /// A shift that lands between two device pixels repaints in full. A copy
    /// rounded to a whole pixel would leave the strip a half pixel off.
    #[test]
    fn a_shift_between_two_device_pixels_repaints_in_full() {
        let plan = surface(800, 600, 1.25)
            .unwrap()
            .on_device(scroll(0.0, -25.0));
        assert!(matches!(plan.content, ContentRepaint::Full));
        assert!(plan.content_strip.is_none());
    }
}
