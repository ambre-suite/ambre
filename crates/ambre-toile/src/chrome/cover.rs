// ambre-toile/src/chrome/cover.rs
// ============================================================================
// The chrome measures what its floating surfaces cover of the board, and
// publishes it. A fit then frames clear of them with no figure of its own
// (`ZOOM-011`). A `ResizeObserver` watches each surface, and the view reads
// the latest measure when a fit reply lands.
// ============================================================================

use std::rc::Rc;

use ambre_core::{Css, Px};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::ui::cover::Cover;

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

/// Run `on_box` with the page box of the element `node` holds, once when it
/// loads and again each time its size changes. A new element under the same
/// reference moves the watch to it; the watch ends with the owner.
fn watch_box(node: NodeRef<html::Div>, on_box: impl Fn(web_sys::DomRect) + 'static) {
    let on_box = Rc::new(on_box);
    let watch = StoredValue::new_local(None::<Watch>);
    Effect::new(move |_| {
        let next = node.get().and_then(|el| {
            let el: web_sys::Element = el.unchecked_into();
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

/// Write one side of the cover, and only when it moved, so a steady surface
/// wakes nothing.
fn publish(
    cover: RwSignal<Cover>,
    side: fn(&mut Cover) -> &mut Option<Px<Css>>,
    value: Option<Px<Css>>,
) {
    let moved = cover
        .try_get_untracked()
        .is_some_and(|mut c| *side(&mut c) != value);
    if moved {
        cover.update(|c| *side(c) = value);
    }
}

fn palette_side(c: &mut Cover) -> &mut Option<Px<Css>> {
    &mut c.palette_right
}

fn bar_side(c: &mut Cover) -> &mut Option<Px<Css>> {
    &mut c.bar_reach
}

/// Publish the right edge of the tool palette `node` holds, and clear it when
/// the palette leaves the page or hides.
pub(super) fn watch_palette(node: NodeRef<html::Div>, cover: RwSignal<Cover>) {
    watch_box(node, move |rect| {
        let right = shown(&rect).then(|| Px::<Css>::new(rect.right() as f32));
        publish(cover, palette_side, right);
    });
    on_cleanup(move || publish(cover, palette_side, None));
}

/// Publish how far the selection bar `node` holds reaches above its anchor,
/// the selection's top edge at `anchor_top` on the page. Cleared when the bar
/// leaves the page.
pub(super) fn watch_bar(
    node: NodeRef<html::Div>,
    anchor_top: impl Fn() -> Option<f32> + 'static,
    cover: RwSignal<Cover>,
) {
    watch_box(node, move |rect| {
        let reach = anchor_top()
            .filter(|_| shown(&rect))
            .map(|top| Px::<Css>::new(top - rect.top() as f32));
        publish(cover, bar_side, reach);
    });
    on_cleanup(move || publish(cover, bar_side, None));
}

/// Clear the bar's reach: the bar is not on the page.
pub(super) fn clear_bar(cover: RwSignal<Cover>) {
    publish(cover, bar_side, None);
}
