//! Reactive + DOM tests for the shared chrome toolkit (Epic `ET`,
//! `ambre-spec-architecture.md` §4 "wasm" tier, `ambre-spec-testing.md`).
//!
//! The chrome components are leptos CSR built on `web-sys`/`wasm-bindgen`: the
//! reactive store and the mounted DOM only exist on the wasm target. So these
//! run under the `wasm-bindgen-test-runner`, wrapped by
//! `wasm-pack test --headless --chrome crates/ambre-chrome` (`just test-wasm`).
//!
//! Two belts (mirrors the trame chrome suite):
//!   - the **reactive belt** -- `CommandStates` over real leptos signals, no DOM;
//!   - the **DOM belt** -- components mounted into the real `document`, asserted
//!     through `web-sys` (ARIA roles, the `AppShell` canvas cell, toast text).
//!
//! On host the whole file is gated out (the `cfg` below), so `just test-native`
//! skips it; the pure decision logic (`key_from_dom`, `place_menu`) is covered
//! by the `#[test]`s in `src/`.
#![cfg(target_arch = "wasm32")]

use ambre_chrome::{
    AppShell, CommandStates, Header, StatusBar, ToastHost, ToastKind, Toasts, Toolbar,
};
use ambre_commands::CommandId;
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const A: CommandId = CommandId("test.a");
const B: CommandId = CommandId("test.b");

// ── Reactive belt: CommandStates (no DOM) ───────────────────────────────────

#[wasm_bindgen_test]
fn command_states_defaults() {
    let owner = Owner::new();
    owner.set();
    let store = CommandStates::new(&[A, B]);
    // Toggles start `false`, enabled starts `true` (actionable until told else).
    assert!(!store.toggled_signal(A).unwrap().get_untracked());
    assert!(store.enabled_signal(A).unwrap().get_untracked());
}

#[wasm_bindgen_test]
fn command_states_single_writer_propagates() {
    let owner = Owner::new();
    owner.set();
    let store = CommandStates::new(&[A, B]);

    store.set_toggled(A, true);
    store.set_enabled(B, false);
    assert!(store.toggled_signal(A).unwrap().get_untracked());
    assert!(
        !store.toggled_signal(B).unwrap().get_untracked(),
        "B toggle untouched"
    );
    assert!(!store.enabled_signal(B).unwrap().get_untracked());
    assert!(
        store.enabled_signal(A).unwrap().get_untracked(),
        "A enabled untouched"
    );
}

#[wasm_bindgen_test]
fn command_states_unknown_id_is_noop() {
    let owner = Owner::new();
    owner.set();
    let store = CommandStates::new(&[A]);
    assert!(store.toggled_signal(B).is_none());
    assert!(store.enabled_signal(B).is_none());
    // Writing an unregistered id must not panic.
    store.set_toggled(B, true);
    store.set_enabled(B, false);
}

// ── DOM belt ────────────────────────────────────────────────────────────────

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `f` into a detached container appended to the body, returning it so the
/// test can query and then drop it. `forget()` keeps the view alive like
/// `mount_to_body`, but scoped to a container we control.
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

#[wasm_bindgen_test]
fn toast_host_is_an_aria_live_region() {
    let owner = Owner::new();
    owner.set();
    let toasts = Toasts::new();
    let root = mount_in_container(move || view! { <ToastHost toasts=toasts/> });

    let host = root.query_selector(".ambre-toasts").unwrap().unwrap();
    assert_eq!(host.get_attribute("role").as_deref(), Some("status"));
    assert_eq!(host.get_attribute("aria-live").as_deref(), Some("polite"));

    root.remove();
}

#[wasm_bindgen_test]
fn toast_push_renders_message_with_kind_class() {
    let owner = Owner::new();
    owner.set();
    let toasts = Toasts::new();
    // Push before mounting so the initial `<For>` render includes the toast
    // synchronously (a post-mount push flushes on a later microtask, which a
    // straight-line assertion would race).
    toasts.push(ToastKind::Error, "boom");
    let root = mount_in_container(move || view! { <ToastHost toasts=toasts/> });

    let toast = root.query_selector(".ambre-toast").unwrap().unwrap();
    assert!(toast.class_name().contains("error"), "error accent class");
    assert_eq!(toast.text_content().as_deref(), Some("boom"));

    root.remove();
}

#[wasm_bindgen_test]
fn app_shell_owns_the_named_canvas_cell() {
    let owner = Owner::new();
    owner.set();
    let root = mount_in_container(|| {
        view! {
            <AppShell canvas_id="ambre-canvas">
                <Header slot>"H"</Header>
                <Toolbar slot>"T"</Toolbar>
                <StatusBar slot>"S"</StatusBar>
            </AppShell>
        }
    });

    // The shell renders the grid regions and the canvas the host attaches to.
    assert!(root.query_selector(".ambre-shell").unwrap().is_some());
    let canvas = root.query_selector("#ambre-canvas").unwrap().unwrap();
    assert_eq!(canvas.tag_name(), "CANVAS");
    assert!(root.query_selector(".ambre-statusbar").unwrap().is_some());

    root.remove();
}
