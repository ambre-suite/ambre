// ambre-chrome/src/cover.rs
// ============================================================================
// Each floating surface of the shell measures what it covers of the canvas.
// It publishes the measure into the one cover of the page (`CHROME-171`). A fit then
// frames clear of the surfaces with no figure of its own (`ZOOM-011`). A
// `ResizeObserver` watches each surface, and the view reads the latest
// measure when a fit lands. The model is `ambre_client_core::cover`.
// ============================================================================

use std::rc::Rc;

use ambre_client_core::cover::{Clearance, Cover, Covered, PageBox, Surface};
use ambre_core::{Css, Point, Px};
use leptos::prelude::*;
use leptos::tachys::html::element::ElementType;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

/// One element under a `ResizeObserver`, disconnected when dropped.
struct Watch {
    observer: web_sys::ResizeObserver,
    _on_resize: Closure<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}

/// The element `node` holds, as a DOM element.
fn element_of<E>(node: NodeRef<E>) -> Option<web_sys::Element>
where
    E: ElementType,
    E::Output: JsCast + Clone + 'static,
{
    node.get().map(JsCast::unchecked_into)
}

/// Run `on_box` with the page box of the element `node` holds, once when it
/// loads and again each time its size changes. A new element under the same
/// reference moves the watch to it; the watch ends with the owner.
fn watch_box<E>(node: NodeRef<E>, on_box: impl Fn(web_sys::DomRect) + 'static)
where
    E: ElementType,
    E::Output: JsCast + Clone + 'static,
{
    let on_box = Rc::new(on_box);
    let watch = StoredValue::new_local(None::<Watch>);
    Effect::new(move |_| {
        let next = element_of(node).and_then(|el| {
            let target = el.clone();
            let on_box = Rc::clone(&on_box);
            let on_resize =
                Closure::<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>::new(move |_, _| {
                    on_box(target.get_bounding_client_rect())
                });
            let observer = web_sys::ResizeObserver::new(on_resize.as_ref().unchecked_ref()).ok()?;
            observer.observe(&el);
            Some(Watch {
                observer,
                _on_resize: on_resize,
            })
        });
        watch.set_value(next);
    });
}

/// A measured box that shows: a box of no size is a surface the page hides.
fn shown(rect: &web_sys::DomRect) -> bool {
    rect.width() > 0.0 && rect.height() > 0.0
}

/// A length the DOM measures in `f64`, as a CSS pixel, which is an `f32`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a coordinate of the page fits an f32"
)]
fn css(v: f64) -> Px<Css> {
    Px::<Css>::new(v as f32)
}

/// The page box of a measured rectangle.
#[must_use]
pub fn page_box(rect: &web_sys::DomRect) -> PageBox {
    PageBox {
        top_left: Point::new(css(rect.left()), css(rect.top())),
        bottom_right: Point::new(css(rect.right()), css(rect.bottom())),
    }
}

/// The strips of the page's canvas that a fit keeps clear of, as each
/// floating surface last measured itself (`ZOOM-011`). A canvas not mounted
/// yet has nothing over it.
#[must_use]
pub fn canvas_clearance(cover: &Cover) -> Clearance {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".ambre-canvas").ok().flatten())
        .map(|el| cover.clearance(page_box(&el.get_bounding_client_rect())))
        .unwrap_or_default()
}

/// Write what `surface` covers, and only when it moved, so a steady surface
/// wakes nothing.
fn publish(cover: RwSignal<Cover>, surface: Surface, value: Option<Covered>) {
    let moved = cover
        .try_get_untracked()
        .is_some_and(|c| c.of(surface) != value);
    if moved {
        cover.update(|c| c.set(surface, value));
    }
}

/// Publish the page box of a surface that keeps its place while the view
/// moves, from the rectangle `rect` measured of it.
fn publish_fixed(cover: RwSignal<Cover>, surface: Surface, rect: &web_sys::DomRect) {
    let covered = shown(rect).then(|| Covered::Fixed(page_box(rect)));
    publish(cover, surface, covered);
}

/// Measure again the surface `node` holds, which keeps its place while the
/// view moves. A surface that moves without a change of size calls it, as
/// the call bubble does when it rests in another corner.
pub fn measure_fixed<E>(node: NodeRef<E>, surface: Surface, cover: RwSignal<Cover>)
where
    E: ElementType,
    E::Output: JsCast + Clone + 'static,
{
    match node.get_untracked() {
        Some(el) => {
            let el: web_sys::Element = el.unchecked_into();
            publish_fixed(cover, surface, &el.get_bounding_client_rect());
        }
        None => publish(cover, surface, None),
    }
}

/// Publish the page box of the surface `node` holds. The surface keeps its
/// place while the view moves: the creation bar, the context panel, the call
/// bubble. The box follows a change of size of the surface or of the window,
/// and clears when the surface hides or leaves the page.
pub fn watch_fixed<E>(node: NodeRef<E>, surface: Surface, cover: RwSignal<Cover>)
where
    E: ElementType,
    E::Output: JsCast + Clone + 'static,
{
    watch_box(node, move |rect| publish_fixed(cover, surface, &rect));
    // A window that resizes moves a surface anchored to its edge, and may
    // leave its size alone.
    let resize = window_event_listener(leptos::ev::resize, move |_| {
        measure_fixed(node, surface, cover);
    });
    on_cleanup(move || {
        resize.remove();
        publish(cover, surface, None);
    });
}

/// Publish how far the context bar `node` holds reaches above its anchor,
/// the selection's top edge at `anchor_top` on the page. Cleared when the bar
/// hides or leaves the page.
pub fn watch_above<E>(
    node: NodeRef<E>,
    anchor_top: impl Fn() -> Option<Px<Css>> + 'static,
    cover: RwSignal<Cover>,
) where
    E: ElementType,
    E::Output: JsCast + Clone + 'static,
{
    watch_box(node, move |rect| {
        let reach = anchor_top()
            .filter(|_| shown(&rect))
            .map(|top| Covered::Above(top - css(rect.top())));
        publish(cover, Surface::ContextBar, reach);
    });
    on_cleanup(move || publish(cover, Surface::ContextBar, None));
}

/// Clear what `surface` covers: it is not on the page.
pub fn clear(cover: RwSignal<Cover>, surface: Surface) {
    publish(cover, surface, None);
}
