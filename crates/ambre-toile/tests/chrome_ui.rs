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

use ambre_bus::UserId;
use ambre_bus::app::toile::TextDelta;
use ambre_canvas::PerfSnapshot;
use ambre_chrome::ContextMenu;
use ambre_commands::CommandId;
use ambre_toile::chrome::{
    FloatBar, PresenterIntent, PresenterPanel, SelStats, SelectionToolbar, StatusBar, ToolPalette,
    TopMenuBar,
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

// -- Presenter overlay panel (toile-features §1.D), input ---------------------

/// Mount a `PresenterPanel` with the given initial state, returning the container
/// and the signal that captures emitted intents. The input signals are created
/// here and moved into the view, so they stay alive for the panel's lifetime.
fn mount_presenter(
    open: bool,
    index: usize,
    count: usize,
    notes: &str,
) -> (web_sys::Element, RwSignal<Option<PresenterIntent>>) {
    mount_presenter_follow(open, index, count, notes, false, None, None)
}

/// Mount a `PresenterPanel` with full "Follow presenter" state (§1.B): the
/// broadcast toggle, the active follow target, and the latest broadcasting peer
/// (which drives whether the Follow button renders).
fn mount_presenter_follow(
    open: bool,
    index: usize,
    count: usize,
    notes: &str,
    broadcasting: bool,
    following: Option<UserId>,
    presenter_peer: Option<UserId>,
) -> (web_sys::Element, RwSignal<Option<PresenterIntent>>) {
    let open = RwSignal::new(open);
    let index = RwSignal::new(index);
    let count = RwSignal::new(count);
    let notes = RwSignal::new(notes.to_owned());
    let broadcasting = RwSignal::new(broadcasting);
    let following = RwSignal::new(following);
    let presenter_peer = RwSignal::new(presenter_peer);
    let fired = RwSignal::new(None::<PresenterIntent>);
    let on_intent = Callback::new(move |i: PresenterIntent| fired.set(Some(i)));
    let root = mount_in_container(move || {
        view! {
            <PresenterPanel
                open=open
                index=index
                count=count
                notes=notes
                broadcasting=broadcasting
                following=following
                presenter_peer=presenter_peer
                on_intent=on_intent
            />
        }
    });
    (root, fired)
}

/// Set a textarea's value and dispatch a bubbling `input` event, the shape leptos
/// `on:input` reads through `event_target_value`.
fn type_into(root: &web_sys::Element, aria: &str, value: &str) {
    let el = root
        .query_selector(&format!("[aria-label=\"{aria}\"]"))
        .unwrap()
        .expect("textarea present");
    let area: web_sys::HtmlTextAreaElement = el.dyn_into().unwrap();
    area.set_value(value);
    let ev = web_sys::Event::new("input").unwrap();
    area.dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
fn presenter_panel_hidden_when_closed() {
    let owner = Owner::new();
    owner.set();
    let (root, _fired) = mount_presenter(false, 0, 3, "");
    // Closed: the panel renders nothing, so the notes editor is absent.
    assert!(
        root.query_selector("[aria-label=\"Speaker notes\"]")
            .unwrap()
            .is_none()
    );
    root.remove();
}

#[wasm_bindgen_test]
fn presenter_panel_counter_is_one_based() {
    let owner = Owner::new();
    owner.set();
    let (root, _fired) = mount_presenter(true, 2, 10, "");
    let count = root
        .query_selector(".ambre-presenter-count")
        .unwrap()
        .unwrap();
    assert_eq!(count.text_content().as_deref(), Some("3 / 10"));
    root.remove();
}

#[wasm_bindgen_test]
fn presenter_nav_buttons_fire_intents() {
    let owner = Owner::new();
    owner.set();
    let (root, fired) = mount_presenter(true, 1, 3, "");

    assert!(click_by_aria(&root, "Next slide"));
    assert_eq!(fired.get_untracked(), Some(PresenterIntent::Next));

    assert!(click_by_aria(&root, "Previous slide"));
    assert_eq!(fired.get_untracked(), Some(PresenterIntent::Prev));

    assert!(click_by_aria(&root, "Exit presentation"));
    assert_eq!(fired.get_untracked(), Some(PresenterIntent::Exit));

    root.remove();
}

#[wasm_bindgen_test]
fn presenter_notes_edit_fires_minimal_delta() {
    let owner = Owner::new();
    owner.set();
    // Seeded with "abc"; appending "d" must emit the single-splice insert, not a
    // full-text replace (toile-features §1.D char-CRDT).
    let (root, fired) = mount_presenter(true, 0, 1, "abc");
    type_into(&root, "Speaker notes", "abcd");
    assert_eq!(
        fired.get_untracked(),
        Some(PresenterIntent::EditNotes(TextDelta {
            at: 3,
            delete: 0,
            insert: "d".to_owned(),
        }))
    );
    root.remove();
}

#[wasm_bindgen_test]
fn presenter_broadcast_button_fires_toggle() {
    let owner = Owner::new();
    owner.set();
    // Broadcasting off: clicking the button requests turning it on (§1.B).
    let (root, fired) = mount_presenter_follow(true, 0, 3, "", false, None, None);
    assert!(click_by_aria(&root, "Broadcast position"));
    assert_eq!(
        fired.get_untracked(),
        Some(PresenterIntent::ToggleBroadcast(true))
    );
    root.remove();
}

#[wasm_bindgen_test]
fn presenter_follow_button_hidden_without_presenter() {
    let owner = Owner::new();
    owner.set();
    // No peer is broadcasting -> the Follow affordance is absent (§1.B).
    let (root, _fired) = mount_presenter_follow(true, 0, 3, "", false, None, None);
    assert!(
        root.query_selector("[aria-label=\"Follow presenter\"]")
            .unwrap()
            .is_none()
    );
    root.remove();
}

#[wasm_bindgen_test]
fn presenter_follow_button_fires_with_peer() {
    let owner = Owner::new();
    owner.set();
    let peer = UserId::new();
    // A peer is broadcasting and we are not following: clicking follows it (§1.B).
    let (root, fired) = mount_presenter_follow(true, 0, 3, "", false, None, Some(peer));
    assert!(click_by_aria(&root, "Follow presenter"));
    assert_eq!(
        fired.get_untracked(),
        Some(PresenterIntent::Follow(Some(peer)))
    );
    root.remove();

    // Already following the peer: clicking stops following (emits `Follow(None)`).
    let owner = Owner::new();
    owner.set();
    let (root, fired) = mount_presenter_follow(true, 0, 3, "", false, Some(peer), Some(peer));
    assert!(click_by_aria(&root, "Follow presenter"));
    assert_eq!(fired.get_untracked(), Some(PresenterIntent::Follow(None)));
    root.remove();
}

// Note: the "slide change re-seeds the textarea" path is a deferred `Effect` in
// `PresenterPanel`. Asserting it needs the reactive executor to flush a post-mount
// signal update, which the bare wasm-bindgen-test harness does not drive (no async
// spawner installed by `mount_to`), so it is covered by the pure `notes_delta`
// model + the app-level wiring rather than a flaky async DOM assertion here.
