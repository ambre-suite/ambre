//! DOM tests for the Toile chrome (Epic `ET`, `ambre-spec-architecture.md` §4
//! "wasm" tier) -- the "wasm render / input tests" DoD item of `toile-ui §13`.
//!
//! The chrome (`ambre_toile::chrome`) is `cfg(target_arch = "wasm32")` -- leptos
//! CSR + DOM (`web-sys`/`wasm-bindgen`) that does not compile on the host. So its
//! tests run under the wasm target via the `wasm-bindgen-test-runner`, wrapped by
//! `wasm-pack test --headless --chrome crates/ambre-toile` (`just test-wasm`).
//!
//! The pure models are host-tested in `ui::{status,menu,menu_bar,palette}`; this
//! belt mounts the real components and asserts the composed DOM plus input:
//! - the selection status band (§8bis) -- aggregates + shared zoom/fps chips;
//! - the right-click context menu (§5ter) end-to-end -- the entry rows a
//!   [`CtxScope`] renders, and that clicking a row fires its `CommandId`;
//! - the tool palette (§5quinquies.A) -- a click switches the shared active tool;
//! - the top menu bar zoom cluster (§5quinquies.E) and the floating selection
//!   toolbar (§5quinquies.D) -- a click fires its `CommandId` on the shared sink.
//!
//! On the host target the whole file is empty (the `cfg` gate below), so the
//! native tier (`just test-native`) skips it cleanly.
#![cfg(target_arch = "wasm32")]

use ambre_canvas::PerfSnapshot;
use ambre_chrome::ContextMenu;
use ambre_commands::CommandId;
use ambre_toile::chrome::{
    FloatBar, SelStats, SelectionToolbar, StatusBar, ToolPalette, TopMenuBar,
};
use ambre_toile::ui::Tool;
use ambre_toile::ui::menu::{self, CtxScope};
use ambre_toile::ui::menu_bar::ZOOM_IN;
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

// -- Context menu (§5ter) -----------------------------------------------------
// The pure `CtxScope -> entries` mapping and the `CommandId -> ToileCommand`
// resolver are host-tested in `ui::menu`; here we mount the real
// `ambre_chrome::ContextMenu` fed by `chrome::context_entries_for` and assert
// the DOM rows a scope renders, plus that clicking a row fires its `CommandId`
// on the shared `on_command` sink (the exact bridge the shell wires, `app_root`).

/// Mount a `ContextMenu` opened at a fixed point, fed by the entries a `scope`
/// renders. Returns the container and the signal the menu's `on_command` writes
/// (so a test can click a row and read back the fired `CommandId`).
fn mount_ctx_menu(scope: CtxScope) -> (web_sys::Element, RwSignal<Option<CommandId>>) {
    let entries = RwSignal::new(ambre_toile::chrome::context_entries_for(&scope));
    let pos = RwSignal::new(Some((10.0, 10.0)));
    let fired = RwSignal::new(None::<CommandId>);
    let on_command = Callback::new(move |id: CommandId| fired.set(Some(id)));
    let root = mount_in_container(
        move || view! { <ContextMenu pos=pos entries=entries on_command=on_command /> },
    );
    (root, fired)
}

/// The visible labels of the mounted context-menu item rows, top to bottom
/// (separators carry no label and are skipped).
fn ctx_item_labels(root: &web_sys::Element) -> Vec<String> {
    let items = root.query_selector_all(".ambre-ctx-item").unwrap();
    (0..items.length())
        .filter_map(|i| items.item(i))
        .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
        .filter_map(|el| el.query_selector(".ambre-ctx-label").ok().flatten())
        .filter_map(|lbl| lbl.text_content())
        .collect()
}

/// Click the context-menu row whose label matches `label`; returns false if no
/// such row is mounted.
fn click_ctx_item(root: &web_sys::Element, label: &str) -> bool {
    let items = root.query_selector_all(".ambre-ctx-item").unwrap();
    for i in 0..items.length() {
        let Some(el) = items
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let matches = el
            .query_selector(".ambre-ctx-label")
            .ok()
            .flatten()
            .and_then(|l| l.text_content())
            .is_some_and(|t| t == label);
        if matches {
            el.dyn_into::<web_sys::HtmlElement>().unwrap().click();
            return true;
        }
    }
    false
}

/// Click the first element carrying `aria-label`; returns false if none exists.
fn click_by_aria(root: &web_sys::Element, aria: &str) -> bool {
    match root
        .query_selector(&format!("[aria-label=\"{aria}\"]"))
        .unwrap()
    {
        Some(el) => {
            el.dyn_into::<web_sys::HtmlElement>().unwrap().click();
            true
        }
        None => false,
    }
}

#[wasm_bindgen_test]
fn context_menu_empty_canvas_shows_only_paste_and_select_all() {
    let owner = Owner::new();
    owner.set();
    // The empty-canvas scope offers exactly Paste + Select all (§5ter).
    let (root, fired) = mount_ctx_menu(CtxScope::default());
    assert_eq!(ctx_item_labels(&root), vec!["Paste", "Select all"]);

    assert!(click_ctx_item(&root, "Select all"));
    assert_eq!(fired.get_untracked(), Some(menu::SELECT_ALL));

    root.remove();
}

#[wasm_bindgen_test]
fn context_menu_copy_row_fires_copy_command() {
    let owner = Owner::new();
    owner.set();
    // A single unlocked element: the clipboard group renders; clicking Copy fires
    // the COPY id the view resolves against its selection.
    let scope = CtxScope {
        over_selection: true,
        count: 1,
        any_unlocked: true,
        ..CtxScope::default()
    };
    let (root, fired) = mount_ctx_menu(scope);
    let labels = ctx_item_labels(&root);
    assert!(labels.contains(&"Copy".to_owned()), "labels: {labels:?}");
    assert!(labels.contains(&"Delete".to_owned()), "labels: {labels:?}");
    // A single-element, unlocked, group-less selection offers neither grouping
    // side nor Unlock (the omission-when-inapplicable rule of §5ter).
    assert!(!labels.contains(&"Group".to_owned()), "labels: {labels:?}");
    assert!(!labels.contains(&"Unlock".to_owned()), "labels: {labels:?}");

    assert!(click_ctx_item(&root, "Copy"));
    assert_eq!(fired.get_untracked(), Some(menu::COPY));

    root.remove();
}

#[wasm_bindgen_test]
fn context_menu_multi_selection_offers_group() {
    let owner = Owner::new();
    owner.set();
    let scope = CtxScope {
        over_selection: true,
        count: 2,
        multi: true,
        any_unlocked: true,
        ..CtxScope::default()
    };
    let (root, fired) = mount_ctx_menu(scope);
    assert!(ctx_item_labels(&root).contains(&"Group".to_owned()));

    assert!(click_ctx_item(&root, "Group"));
    assert_eq!(fired.get_untracked(), Some(menu::GROUP));

    root.remove();
}

#[wasm_bindgen_test]
fn context_menu_locked_selection_offers_only_unlock() {
    let owner = Owner::new();
    owner.set();
    // A wholly-locked selection: Unlock applies, Lock is omitted (§5ter).
    let scope = CtxScope {
        over_selection: true,
        count: 1,
        any_locked: true,
        ..CtxScope::default()
    };
    let (root, fired) = mount_ctx_menu(scope);
    let labels = ctx_item_labels(&root);
    assert!(labels.contains(&"Unlock".to_owned()), "labels: {labels:?}");
    assert!(!labels.contains(&"Lock".to_owned()), "labels: {labels:?}");

    assert!(click_ctx_item(&root, "Unlock"));
    assert_eq!(fired.get_untracked(), Some(menu::UNLOCK));

    root.remove();
}

// -- Tool palette (§5quinquies.A), input --------------------------------------

#[wasm_bindgen_test]
fn tool_palette_click_switches_active_tool() {
    let owner = Owner::new();
    owner.set();
    let active = RwSignal::new(Tool::Select);
    let root = mount_in_container(move || view! { <ToolPalette active=active /> });

    // The Select button reflects the active tool through `aria-pressed`.
    let select_btn = root
        .query_selector("[aria-label=\"Select\"]")
        .unwrap()
        .unwrap();
    assert_eq!(
        select_btn.get_attribute("aria-pressed").as_deref(),
        Some("true")
    );

    // Clicking Pen writes the shared active-tool signal the canvas view reads.
    assert!(click_by_aria(&root, "Pen"));
    assert!(matches!(active.get_untracked(), Tool::Pen));

    root.remove();
}

// -- Top menu bar zoom cluster (§5quinquies.E), input -------------------------

#[wasm_bindgen_test]
fn top_menu_bar_zoom_in_fires_command() {
    let owner = Owner::new();
    owner.set();
    let zoom = RwSignal::new(1.0);
    let fired = RwSignal::new(None::<CommandId>);
    let on_command = Callback::new(move |id: CommandId| fired.set(Some(id)));
    let root = mount_in_container(move || view! { <TopMenuBar zoom=zoom on_command=on_command /> });

    assert!(click_by_aria(&root, "Zoom in"));
    assert_eq!(fired.get_untracked(), Some(ZOOM_IN));

    root.remove();
}

// -- Floating selection toolbar (§5quinquies.D), input ------------------------

#[wasm_bindgen_test]
fn selection_toolbar_delete_fires_command() {
    let owner = Owner::new();
    owner.set();
    // A live single-element bar: Delete is always present; Group/Ungroup/Lock are
    // gated by the same flags as the context menu.
    let bar = RwSignal::new(Some(FloatBar {
        x: 40.0,
        y: 20.0,
        multi: false,
        has_group: false,
        any_locked: false,
        any_unlocked: true,
        fill: None,
    }));
    let fired = RwSignal::new(None::<CommandId>);
    let on_command = Callback::new(move |id: CommandId| fired.set(Some(id)));
    let on_fill = Callback::new(move |_c: Option<u32>| {});
    let root = mount_in_container(move || {
        view! { <SelectionToolbar bar=bar on_command=on_command on_fill=on_fill /> }
    });

    assert!(click_by_aria(&root, "Delete"));
    assert_eq!(fired.get_untracked(), Some(menu::DELETE));

    root.remove();
}
