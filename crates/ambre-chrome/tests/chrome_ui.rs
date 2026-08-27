//! Reactive + DOM tests for the shared chrome toolkit (Epic `ET`,
//! `ambre-spec-testing.md`, "wasm" tier).
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
    AppShell, CommandStates, FONT_SIZES, FontSizeSelect, FormatToggle, Header, PerfChip,
    PerfDetail, StatusBar, ToastHost, ToastKind, Toasts, Toolbar, ZoomChip,
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
    let store = CommandStates::new([A, B]);
    // Toggles start `false`, enabled starts `true` (actionable until told else).
    assert!(!store.toggled_signal(A).unwrap().get_untracked());
    assert!(store.enabled_signal(A).unwrap().get_untracked());
}

#[wasm_bindgen_test]
fn command_states_single_writer_propagates() {
    let owner = Owner::new();
    owner.set();
    let store = CommandStates::new([A, B]);

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
    let store = CommandStates::new([A]);
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

// ── Shared status-bar chips (status.rs) ─────────────────────────────────────

#[wasm_bindgen_test]
fn zoom_chip_renders_readout_from_signal_and_starts_collapsed() {
    let owner = Owner::new();
    owner.set();
    // The readout mirrors the live factor (1.25 -> "125 %") and the preset menu
    // is collapsed on mount (only the button shows). Reactive re-render on click
    // flushes on a later microtask, so a straight-line post-click DOM query would
    // race; we assert the synchronous initial render only.
    let zoom = RwSignal::new(1.25_f64);
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom /> });

    let btn = root.query_selector(".statusbar-zoom-btn").unwrap().unwrap();
    assert_eq!(btn.text_content().as_deref(), Some("125 %"));
    assert_eq!(btn.get_attribute("aria-haspopup").as_deref(), Some("menu"));
    assert!(
        root.query_selector(".statusbar-zoom-menu")
            .unwrap()
            .is_none()
    );

    root.remove();
}

#[wasm_bindgen_test]
fn perf_chip_renders_fps_from_signal_and_starts_collapsed() {
    let owner = Owner::new();
    owner.set();
    // The chip shows the live fps ("58 fps") and the app-supplied detail panel is
    // collapsed on mount (toggled open on click, an async re-render).
    let fps = RwSignal::new(58.0_f32);
    let root = mount_in_container(move || {
        view! {
            <PerfChip fps=Signal::derive(move || fps.get())>
                <PerfDetail slot>
                    <div class="test-perf-detail">"detail"</div>
                </PerfDetail>
            </PerfChip>
        }
    });

    let btn = root.query_selector(".statusbar-fps").unwrap().unwrap();
    assert_eq!(btn.text_content().as_deref(), Some("58 fps"));
    assert!(root.query_selector(".test-perf-detail").unwrap().is_none());

    root.remove();
}

// ── Shared text-format controls (format.rs) ─────────────────────────────────

#[wasm_bindgen_test]
fn format_toggle_reflects_pressed_and_reports_clicks() {
    let owner = Owner::new();
    owner.set();
    let pressed = RwSignal::new(false);
    let clicks = RwSignal::new(0u32);
    let root = mount_in_container(move || {
        view! {
            <FormatToggle
                label="Bold"
                icon=icondata::BsTypeBold
                pressed=pressed.into()
                on_toggle=Callback::new(move |()| clicks.update(|c| *c += 1))
            />
        }
    });

    let btn = root.query_selector("button").unwrap().unwrap();
    assert_eq!(btn.get_attribute("aria-label").as_deref(), Some("Bold"));
    // Unpressed: no aria-pressed attribute.
    assert!(btn.get_attribute("aria-pressed").is_none());
    // The signal drives the pressed state (a later microtask re-renders it).
    pressed.set(true);

    // A click reports the intent through the callback (the control is stateless).
    let ev = web_sys::MouseEvent::new("click").unwrap();
    btn.dispatch_event(&ev).unwrap();
    assert_eq!(clicks.get_untracked(), 1);

    root.remove();
}

#[wasm_bindgen_test]
fn font_size_select_lists_presets_plus_auto_entry() {
    let owner = Owner::new();
    owner.set();
    let picked = RwSignal::new(Some(999u16));
    let root = mount_in_container(move || {
        view! {
            <FontSizeSelect
                current=Signal::derive(|| Some(12u16))
                on_pick=Callback::new(move |p: Option<u16>| picked.set(p))
                auto_label="Auto"
            />
        }
    });

    let select = root.query_selector("select").unwrap().unwrap();
    // The unset ("Auto") entry plus every preset size.
    let options = select.get_elements_by_tag_name("option");
    assert_eq!(options.length() as usize, FONT_SIZES.len() + 1);
    assert_eq!(
        select.get_attribute("aria-label").as_deref(),
        Some("Font size")
    );

    root.remove();
}

// ── DOM belt: the command palette (CHROME-008) ──────────────────────────────

fn palette_title(l: &ambre_commands::Locale) -> String {
    let _ = l;
    "Command palette".into()
}
fn hidden_title(l: &ambre_commands::Locale) -> String {
    let _ = l;
    "Recalculate everything".into()
}
fn shown_title(l: &ambre_commands::Locale) -> String {
    let _ = l;
    "Merge cells".into()
}

/// A table whose second command appears in no menu: exactly the case
/// `CHROME-008` is about.
const PALETTE_TABLE: &[ambre_commands::Command] = &[
    ambre_commands::Command {
        id: CommandId("t.palette.open"),
        title: palette_title,
        icon: None,
        default_keybinding: None,
        category: ambre_commands::CommandCategory("general"),
    },
    ambre_commands::Command {
        id: CommandId("t.engine.recalc"),
        title: hidden_title,
        icon: None,
        default_keybinding: None,
        category: ambre_commands::CommandCategory("engine"),
    },
    ambre_commands::Command {
        id: CommandId("t.merge"),
        title: shown_title,
        icon: None,
        default_keybinding: None,
        category: ambre_commands::CommandCategory("structure"),
    },
];

/// The visible labels of the palette rows, top to bottom.
fn palette_labels(root: &web_sys::Element) -> Vec<String> {
    let nodes = root.query_selector_all(".ambre-palette-label").unwrap();
    (0..nodes.length())
        .filter_map(|i| nodes.item(i).and_then(|n| n.text_content()))
        .collect()
}

/// DOM test: an opened palette renders one row per command the table declares,
/// including a command no menu shows (`ambre-spec-chrome.md`, CHROME-008).
///
/// Mounted already open: a reactive re-render flushes on a later microtask, so
/// only the synchronous initial render is asserted -- the same rule the chip
/// tests above follow. What a query does to the list is the pure `filter`, and
/// its assertions are the host tests in `src/palette.rs`.
#[wasm_bindgen_test]
fn palette_lists_every_declared_command() {
    let owner = Owner::new();
    owner.set();
    let open = RwSignal::new(true);
    let root = mount_in_container(move || {
        view! {
            <ambre_chrome::CommandPalette
                open=open
                table=PALETTE_TABLE
                on_command=Callback::new(move |_| {})
            />
        }
    });

    let labels = palette_labels(&root);
    assert_eq!(labels.len(), PALETTE_TABLE.len(), "labels: {labels:?}");
    assert!(
        labels.contains(&"Recalculate everything".to_owned()),
        "a command no menu shows must still be listed: {labels:?}"
    );

    root.remove();
}

/// DOM test: selecting a row invokes that command through the shared sink and
/// closes the overlay. A click handler runs synchronously, so both are readable
/// straight after it.
#[wasm_bindgen_test]
fn palette_row_invokes_its_command_and_closes() {
    let owner = Owner::new();
    owner.set();
    let open = RwSignal::new(true);
    let invoked: RwSignal<Option<CommandId>> = RwSignal::new(None);
    let root = mount_in_container(move || {
        view! {
            <ambre_chrome::CommandPalette
                open=open
                table=PALETTE_TABLE
                on_command=Callback::new(move |id| invoked.set(Some(id)))
            />
        }
    });

    let rows = root.query_selector_all(".ambre-palette-row").unwrap();
    let last: web_sys::HtmlElement = rows.item(rows.length() - 1).unwrap().dyn_into().unwrap();
    last.click();

    assert_eq!(invoked.get_untracked(), Some(CommandId("t.merge")));
    assert!(!open.get_untracked(), "selecting a row closes the palette");

    root.remove();
}
