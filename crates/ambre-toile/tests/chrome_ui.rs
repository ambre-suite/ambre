//! DOM tests for the Toile selection status bar (§8bis, Epic `ET`,
//! `ambre-spec-architecture.md` §4 "wasm" tier).
//!
//! The chrome (`ambre_toile::chrome`) is `cfg(target_arch = "wasm32")` -- leptos
//! CSR + DOM (`web-sys`/`wasm-bindgen`) that does not compile on the host. So its
//! tests run under the wasm target via the `wasm-bindgen-test-runner`, wrapped by
//! `wasm-pack test --headless --chrome crates/ambre-toile` (`just test-wasm`).
//!
//! The pure count/bounds formatting is host-tested in `ui::status`; this belt
//! mounts the real component and asserts the composed band -- the left-side
//! aggregates plus the shared zoom + fps chips (§8bis).
//!
//! On the host target the whole file is empty (the `cfg` gate below), so the
//! native tier (`just test-native`) skips it cleanly.
#![cfg(target_arch = "wasm32")]

use ambre_canvas::PerfSnapshot;
use ambre_toile::chrome::{SelStats, StatusBar};
use ambre_toile::ui::status::Bounds;
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `f` into a detached container appended to the body, returning it so the
/// test can query and then drop it.
fn mount_in_container<F, N>(f: F) -> web_sys::Element
where
    F: FnOnce() -> N + 'static,
    N: IntoView + 'static,
{
    let container = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&container).unwrap();
    let target: web_sys::HtmlElement = container.clone().dyn_into().unwrap();
    mount_to(target, f).forget();
    container
}

fn mount_bar(stats: Option<SelStats>, zoom: f64, fps: f32) -> (web_sys::Element, RwSignal<f64>) {
    let stats = RwSignal::new(stats);
    let zoom = RwSignal::new(zoom);
    let perf = RwSignal::new(PerfSnapshot {
        fps,
        ..PerfSnapshot::default()
    });
    let root = mount_in_container(move || view! { <StatusBar stats=stats perf=perf zoom=zoom /> });
    (root, zoom)
}

#[wasm_bindgen_test]
fn multi_selection_shows_count_bounds_and_chips() {
    let owner = Owner::new();
    owner.set();
    let stats = SelStats {
        count: 3,
        bounds: Some(Bounds {
            x: 10.0,
            y: 20.0,
            w: 40.0,
            h: 60.0,
        }),
    };
    let (root, _zoom) = mount_bar(Some(stats), 1.5, 60.0);

    // Left side: the app label, the count and the combined-bounds captions.
    let text = root
        .query_selector(".statusbar-stats")
        .unwrap()
        .unwrap()
        .text_content()
        .unwrap();
    assert!(text.contains("3 elements"), "count caption: {text}");
    assert!(text.contains("x 10"), "bounds caption: {text}");
    assert!(text.contains("w 40"), "bounds caption: {text}");

    // Right side: the shared interactive zoom + fps chips read their signals.
    let zoom_btn = root.query_selector(".statusbar-zoom-btn").unwrap().unwrap();
    assert_eq!(zoom_btn.text_content().as_deref(), Some("150 %"));
    let fps_btn = root.query_selector(".statusbar-fps").unwrap().unwrap();
    assert_eq!(fps_btn.text_content().as_deref(), Some("60 fps"));

    root.remove();
}

#[wasm_bindgen_test]
fn empty_selection_shows_only_the_chips() {
    let owner = Owner::new();
    owner.set();
    // No selection reply yet: the aggregates area is empty, but the zoom chip
    // (and fps chip) still render (§8bis).
    let (root, _zoom) = mount_bar(None, 1.0, 30.0);

    let stats = root.query_selector(".statusbar-stats").unwrap().unwrap();
    assert_eq!(
        stats.text_content().as_deref().unwrap_or("").trim(),
        "",
        "no aggregates on an empty selection"
    );
    assert!(
        root.query_selector(".statusbar-zoom-btn")
            .unwrap()
            .is_some()
    );
    assert!(root.query_selector(".statusbar-fps").unwrap().is_some());

    root.remove();
}

#[wasm_bindgen_test]
fn single_selection_shows_count_without_bounds() {
    let owner = Owner::new();
    owner.set();
    // A lone element: the count shows, but no combined-bounds caption (§8bis).
    let stats = SelStats {
        count: 1,
        bounds: Some(Bounds {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        }),
    };
    let (root, _zoom) = mount_bar(Some(stats), 1.0, 60.0);

    let text = root
        .query_selector(".statusbar-stats")
        .unwrap()
        .unwrap()
        .text_content()
        .unwrap();
    assert!(text.contains("1 element"), "count caption: {text}");
    assert!(!text.contains("w "), "no bounds for a lone element: {text}");

    root.remove();
}
