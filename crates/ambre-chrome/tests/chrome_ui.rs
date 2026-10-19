//! Reactive + DOM tests for the shared chrome toolkit (Epic `ET`,
//! `ambre-spec-testing.md`, "wasm" tier).
//!
//! The chrome components are leptos CSR built on `web-sys`/`wasm-bindgen`: the
//! reactive store and the mounted DOM only exist on the wasm target. So these run
//! under the `wasm-bindgen-test-runner`, wrapped by `wasm-pack test --headless
//! --chrome crates/ambre-chrome` (`just test-wasm`).
//!
//! Two belts (mirrors the trame chrome suite):
//!   - the **reactive belt** -- `CommandStates` over real leptos signals, no DOM;
//!   - the **DOM belt** -- components mounted into the real `document`, asserted
//!     through `web-sys` (ARIA roles, the `AppShell` canvas cell, toast text).
//!
//! On host the whole file is gated out (the `cfg` below), so `just test-native`
//! skips it. The pure decision logic (`key_from_dom`, `place_menu`) is covered by
//! the `#[test]`s in `src/`.
#![cfg(target_arch = "wasm32")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_chrome::{
    AppShell, CommandStates, ContextMenu, FONT_SIZES, FontSizeSelect, FormatBar, FormatControl,
    FormatToggle, Header, MenuEntry, PerfChip, PerfDetail, Pressed, StatusBar, ToastHost,
    ToastKind, Toasts, Toolbar, ZoomChip, ZoomFactor,
};
use ambre_commands::CommandId;
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const A: CommandId = CommandId("test.a");
const B: CommandId = CommandId("test.b");

// -- Reactive belt: CommandStates (no DOM) -----------------------------------

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

// -- DOM belt ----------------------------------------------------------------

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
    // synchronously. A post-mount push flushes on a later microtask, which a
    // straight-line assertion would race.
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

// -- Shared status-bar chips (status.rs) -------------------------------------

/// The readout follows the factor after a change, not only at mount
/// (`CHROME-086`).
///
/// A re-render lands on a later microtask, so the change is read after one
/// reactive tick.
#[wasm_bindgen_test]
async fn the_zoom_readout_follows_its_signal() {
    let owner = Owner::new();
    owner.set();
    let zoom = RwSignal::new(ZoomFactor::within(1.25, 0.5, 2.0));
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom /> });
    let btn = root.query_selector(".statusbar-zoom-btn").unwrap().unwrap();
    assert_eq!(btn.text_content().as_deref(), Some("125 %"));

    zoom.set(ZoomFactor::within(2.0, 0.5, 2.0));
    leptos::task::tick().await;
    assert_eq!(btn.text_content().as_deref(), Some("200 %"));

    root.remove();
}

/// The fps readout follows its signal after a change (`CHROME-086`).
#[wasm_bindgen_test]
async fn the_fps_readout_follows_its_signal() {
    let owner = Owner::new();
    owner.set();
    let fps = RwSignal::new(58.0_f32);
    let root = mount_in_container(move || {
        view! { <PerfChip fps=Signal::derive(move || fps.get()) /> }
    });
    let btn = root.query_selector(".statusbar-fps").unwrap().unwrap();
    assert_eq!(btn.text_content().as_deref(), Some("58 fps"));

    fps.set(30.0);
    leptos::task::tick().await;
    assert_eq!(btn.text_content().as_deref(), Some("30 fps"));

    root.remove();
}

#[wasm_bindgen_test]
fn zoom_chip_renders_readout_from_signal_and_starts_collapsed() {
    let owner = Owner::new();
    owner.set();
    // The readout mirrors the live factor (1.25 -> "125 %") and the preset menu
    // is collapsed on mount (only the button shows).
    let zoom = RwSignal::new(ZoomFactor::within(1.25, 0.5, 2.0));
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

/// Resolve after `ms` milliseconds, on a later task.
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let window = web_sys::window().unwrap();
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

// -- The debouncer (debounce.rs) ---------------------------------------------

/// A burst of calls runs once, after the burst (`CHROME-015`), and the run is
/// the burst's last value (`CHROME-090`).
///
/// The burst ends before the delay does. A debouncer that dropped the tail
/// would leave the control showing a value the document never received.
#[wasm_bindgen_test]
async fn a_burst_runs_once_with_its_last_value() {
    let owner = Owner::new();
    owner.set();
    let runs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let debouncer = ambre_chrome::Debouncer::new();
    for value in 0..5 {
        let runs = runs.clone();
        debouncer.call(std::time::Duration::from_millis(30), move || {
            runs.borrow_mut().push(value);
        });
    }
    assert!(runs.borrow().is_empty(), "nothing runs during the burst");

    sleep(100).await;
    assert_eq!(*runs.borrow(), vec![4], "one run, with the last value");
}

// -- Shared text-format controls (format.rs) ---------------------------------

/// A click leaves as a report and the pressed state does not move until its
/// signal does (`CHROME-087`).
///
/// The signal is read-only to the control. This test reads the DOM after the
/// click, so a control that showed its own guess would fail here.
#[wasm_bindgen_test]
async fn a_toggle_reports_its_click_and_waits_for_the_value() {
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
    let btn: web_sys::HtmlElement = root
        .query_selector("button")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();

    btn.click();
    leptos::task::tick().await;
    assert_eq!(clicks.get_untracked(), 1, "the click is reported");
    assert!(!pressed.get_untracked(), "the control wrote no state");
    assert_eq!(
        btn.get_attribute("aria-pressed").as_deref(),
        Some("false"),
        "nor showed one"
    );

    // The value comes back from the Logic thread, and only then is shown.
    pressed.set(true);
    leptos::task::tick().await;
    assert_eq!(btn.get_attribute("aria-pressed").as_deref(), Some("true"));

    root.remove();
}

#[wasm_bindgen_test]
async fn format_toggle_reflects_pressed_and_reports_clicks() {
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
    // Unpressed still reads as a toggle: the state is "false", not absent
    // (`CHROME-066`).
    assert_eq!(btn.get_attribute("aria-pressed").as_deref(), Some("false"));
    // The signal drives the pressed state (a later microtask re-renders it).
    pressed.set(true);

    // A click reports the intent through the callback (the control is stateless).
    let ev = web_sys::MouseEvent::new("click").unwrap();
    btn.dispatch_event(&ev).unwrap();
    assert_eq!(clicks.get_untracked(), 1);

    leptos::task::tick().await;
    assert_eq!(btn.get_attribute("aria-pressed").as_deref(), Some("true"));
    pressed.set(false);
    leptos::task::tick().await;
    assert_eq!(btn.get_attribute("aria-pressed").as_deref(), Some("false"));
    root.remove();
}

/// `CHROME-105`: the format bar hangs above its anchor, shows each control
/// with its pressed state, and a press reports that control alone.
#[wasm_bindgen_test]
async fn the_format_bar_hangs_its_controls_and_reports_a_press() {
    let owner = Owner::new();
    owner.set();
    let pressed = RwSignal::new(Vec::<&'static str>::new());
    let control = |label: &'static str, state: Pressed| FormatControl {
        label: label.to_owned(),
        pressed: state,
        on_press: Callback::new(move |()| pressed.update(|p| p.push(label))),
    };
    let controls = vec![
        control("Bold", Pressed::On),
        control("Italic", Pressed::Mixed),
        control("Code", Pressed::Off),
    ];
    let root =
        mount_in_container(move || view! { <FormatBar anchor=(200.0, 100.0) controls=controls /> });

    let bar = root.query_selector("[role=\"toolbar\"]").unwrap().unwrap();
    let style = bar.get_attribute("style").unwrap_or_default();
    assert!(
        style.contains("left: 200px") && style.contains("top: 60px"),
        "{style}"
    );
    let buttons = bar.query_selector_all("button").unwrap();
    let states: Vec<(String, String)> = (0..buttons.length())
        .filter_map(|i| buttons.item(i))
        .filter_map(|b| b.dyn_into::<web_sys::Element>().ok())
        .map(|b| {
            (
                b.get_attribute("aria-label").unwrap_or_default(),
                b.get_attribute("aria-pressed").unwrap_or_default(),
            )
        })
        .collect();
    let want = [("Bold", "true"), ("Italic", "mixed"), ("Code", "false")]
        .map(|(l, p)| (l.to_owned(), p.to_owned()));
    assert_eq!(states, want.to_vec());

    let italic = root
        .query_selector("[aria-label=\"Italic\"]")
        .unwrap()
        .unwrap();
    italic
        .dispatch_event(&web_sys::MouseEvent::new("click").unwrap())
        .unwrap();
    assert_eq!(pressed.get_untracked(), vec!["Italic"]);
    root.remove();
}

// -- Accessibility (menu.rs, ambre-spec-accessibility.md) --------------------

/// The element that holds the focus now.
fn active() -> Option<web_sys::Element> {
    document().active_element()
}

/// Dispatch a bubbling keydown carrying `key` on `target`.
fn press(target: &web_sys::Element, key: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(key);
    init.set_bubbles(true);
    init.set_cancelable(true);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// A menu trigger says whether its menu is open, closed included
/// (`CHROME-070`).
#[wasm_bindgen_test]
async fn a_menu_trigger_reports_open_and_closed() {
    let owner = Owner::new();
    owner.set();
    let zoom = RwSignal::new(ZoomFactor::ONE);
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom /> });
    let btn: web_sys::HtmlElement = root
        .query_selector(".statusbar-zoom-btn")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(btn.get_attribute("aria-expanded").as_deref(), Some("false"));

    btn.click();
    leptos::task::tick().await;
    assert_eq!(btn.get_attribute("aria-expanded").as_deref(), Some("true"));

    btn.click();
    leptos::task::tick().await;
    assert_eq!(btn.get_attribute("aria-expanded").as_deref(), Some("false"));
    root.remove();
}

/// An opened menu is driven by the keyboard alone: it takes the focus, the
/// arrows move it, Enter chooses, and Escape closes (`CHROME-026`).
#[wasm_bindgen_test]
async fn a_menu_is_operable_by_keyboard_alone() {
    let owner = Owner::new();
    owner.set();
    let zoom = RwSignal::new(ZoomFactor::ONE);
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom /> });
    let btn: web_sys::HtmlElement = root
        .query_selector(".statusbar-zoom-btn")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    btn.click();
    leptos::task::tick().await;

    let menu = root
        .query_selector(".statusbar-zoom-menu")
        .unwrap()
        .unwrap();
    let items = menu.query_selector_all("[role=menuitemradio]").unwrap();
    let first = items.item(0).unwrap();
    let second = items.item(1).unwrap();
    assert_eq!(
        active().as_ref(),
        first.dyn_ref(),
        "the menu took the focus"
    );
    // Every option says whether it is the live one, not only the chosen one.
    let unchecked = (0..items.length())
        .filter_map(|i| items.item(i))
        .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
        .filter(|e| e.get_attribute("aria-checked").as_deref() == Some("false"))
        .count();
    assert_eq!(unchecked as u32, items.length() - 1);

    press(&menu, "ArrowDown");
    assert_eq!(active().as_ref(), second.dyn_ref(), "ArrowDown moved it");
    press(&menu, "ArrowUp");
    press(&menu, "ArrowUp");
    let last = items.item(items.length() - 1).unwrap();
    assert_eq!(active().as_ref(), last.dyn_ref(), "ArrowUp wrapped");

    // The presets run high to low, so the last one is the smallest.
    press(&menu, "Enter");
    leptos::task::tick().await;
    assert_eq!(
        zoom.get_untracked().get(),
        0.5,
        "Enter chose the focused preset"
    );
    assert!(
        root.query_selector(".statusbar-zoom-menu")
            .unwrap()
            .is_none()
    );

    btn.click();
    leptos::task::tick().await;
    let menu = root
        .query_selector(".statusbar-zoom-menu")
        .unwrap()
        .unwrap();
    press(&menu, "Escape");
    leptos::task::tick().await;
    assert!(
        root.query_selector(".statusbar-zoom-menu")
            .unwrap()
            .is_none()
    );
    assert_eq!(zoom.get_untracked().get(), 0.5, "Escape chose nothing");
    root.remove();
}

/// A key that closes a menu opened from a trigger, Escape or Enter, gives
/// the focus back to the trigger (`CHROME-027`).
#[wasm_bindgen_test]
async fn a_key_that_closes_a_trigger_menu_returns_the_focus_to_its_trigger() {
    let owner = Owner::new();
    owner.set();
    let zoom = RwSignal::new(ZoomFactor::ONE);
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom /> });
    let btn: web_sys::HtmlElement = root
        .query_selector(".statusbar-zoom-btn")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    for key in ["Escape", "Enter"] {
        btn.focus().unwrap();
        btn.click();
        leptos::task::tick().await;
        let menu = root
            .query_selector(".statusbar-zoom-menu")
            .unwrap()
            .unwrap();
        // A menu opened again takes the focus a task later than the first
        // time. Its list is built, then its node lands, then the effect runs.
        leptos::task::tick().await;
        assert_ne!(active().as_ref(), btn.dyn_ref(), "{key}: the menu took it");
        press(&menu, key);
        leptos::task::tick().await;
        assert_eq!(
            active().as_ref(),
            btn.dyn_ref(),
            "{key}: the trigger has it"
        );
    }
    root.remove();
}

/// The floating menu gives the focus back to what held it when it opened,
/// unless the user moved it elsewhere meanwhile (`CHROME-027`).
#[wasm_bindgen_test]
async fn closing_the_floating_menu_returns_the_focus_to_its_opener() {
    let owner = Owner::new();
    owner.set();
    let pos = RwSignal::new(None::<(f64, f64)>);
    let item = ambre_commands::Command {
        id: CommandId("test.item"),
        title: |_| "Item".to_owned(),
        icon: None,
        keybindings: &[],
        category: ambre_commands::CommandCategory(""),
    };
    let entries = vec![MenuEntry::command(&item, &ambre_commands::Locale::En)];
    let root = mount_in_container(move || {
        view! {
            <button class="test-opener">"Opener"</button>
            <input class="test-elsewhere"/>
            <ContextMenu pos=pos entries=entries on_command=Callback::new(|_| {})/>
        }
    });
    let opener: web_sys::HtmlElement = root
        .query_selector(".test-opener")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    opener.focus().unwrap();
    pos.set(Some((10.0, 10.0)));
    leptos::task::tick().await;
    let menu = root.query_selector(".ambre-ctx-menu").unwrap().unwrap();
    let inside = active().is_some_and(|el| menu.contains(Some(&el)));
    assert!(inside, "the menu took the focus");
    press(&menu, "Escape");
    leptos::task::tick().await;
    assert!(root.query_selector(".ambre-ctx-menu").unwrap().is_none());
    assert_eq!(active().as_ref(), opener.dyn_ref(), "the focus came back");

    pos.set(Some((10.0, 10.0)));
    leptos::task::tick().await;
    let elsewhere: web_sys::HtmlElement = root
        .query_selector(".test-elsewhere")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    elsewhere.focus().unwrap();
    pos.set(None);
    leptos::task::tick().await;
    assert_eq!(
        active().as_ref(),
        elsewhere.dyn_ref(),
        "a focus moved by the user stays"
    );
    root.remove();
}

/// A toast is announced without taking the focus from where it was
/// (`CHROME-065`).
#[wasm_bindgen_test]
async fn a_toast_leaves_the_focus_where_it_was() {
    let owner = Owner::new();
    owner.set();
    let toasts = Toasts::new();
    let root = mount_in_container(move || {
        view! {
            <button class="test-focus">"Here"</button>
            <ToastHost toasts=toasts/>
        }
    });
    let here: web_sys::HtmlElement = root
        .query_selector(".test-focus")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    here.focus().unwrap();

    toasts.push(ToastKind::Error, "saved elsewhere");
    leptos::task::tick().await;
    assert!(root.query_selector(".ambre-toast").unwrap().is_some());
    assert_eq!(active().as_ref(), Some(here.as_ref()));
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

// -- DOM belt: the command palette (CHROME-008) ------------------------------

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
        keybindings: &[],
        category: ambre_commands::CommandCategory("general"),
    },
    ambre_commands::Command {
        id: CommandId("t.engine.recalc"),
        title: hidden_title,
        icon: None,
        keybindings: &[],
        category: ambre_commands::CommandCategory("engine"),
    },
    ambre_commands::Command {
        id: CommandId("t.merge"),
        title: shown_title,
        icon: None,
        keybindings: &[],
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
/// Mounted already open: a reactive re-render flushes on a later microtask.
/// Only the synchronous initial render is asserted, the same rule the chip
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
///
/// The close re-renders on a later microtask, so the test awaits it while
/// its owner lives. An effect left pending would read a disposed signal in
/// the next async test.
#[wasm_bindgen_test]
async fn palette_row_invokes_its_command_and_closes() {
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

    leptos::task::tick().await;
    root.remove();
}

/// DOM test: the entries an application supplies are drawn before the
/// commands. Choosing one hands its key back and closes the overlay, and runs
/// no command.
#[wasm_bindgen_test]
async fn palette_entry_comes_first_and_hands_back_its_key() {
    let owner = Owner::new();
    owner.set();
    let open = RwSignal::new(true);
    let chosen: RwSignal<Option<[u8; 16]>> = RwSignal::new(None);
    let invoked: RwSignal<Option<CommandId>> = RwSignal::new(None);
    let entries = Signal::stored(vec![ambre_chrome::PaletteEntry {
        key: [7; 16],
        label: "budget".to_owned(),
        hint: Some("Channel"),
    }]);
    let root = mount_in_container(move || {
        view! {
            <ambre_chrome::CommandPalette
                open=open
                table=PALETTE_TABLE
                on_command=Callback::new(move |id| invoked.set(Some(id)))
                entries=entries
                on_entry=Callback::new(move |key| chosen.set(Some(key)))
            />
        }
    });

    let labels = palette_labels(&root);
    assert_eq!(labels.len(), PALETTE_TABLE.len() + 1, "labels: {labels:?}");
    assert_eq!(labels[0], "budget");
    let first: web_sys::HtmlElement = root
        .query_selector(".ambre-palette-row")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    first.click();

    assert_eq!(chosen.get_untracked(), Some([7; 16]));
    assert_eq!(invoked.get_untracked(), None);
    assert!(
        !open.get_untracked(),
        "choosing an entry closes the palette"
    );

    leptos::task::tick().await;
    root.remove();
}

// -- The modal (dialog.rs) ---------------------------------------------------

/// A mounted modal, the hold count its `ModalHold` reached, and its element.
struct Modal {
    root: web_sys::Element,
    dialog: web_sys::HtmlDialogElement,
    holds: std::sync::Arc<std::sync::atomic::AtomicI32>,
    /// How many times `on_close` ran.
    heard: RwSignal<u32>,
    /// The mount's handle: dropping it unmounts the modal.
    handle: Option<Box<dyn std::any::Any>>,
}

impl Modal {
    fn holds(&self) -> i32 {
        self.holds.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Resolves on the element's next `close` event, after the dialog's own
    /// handler, which was registered first. Call it before the close.
    fn closed(&self) -> wasm_bindgen_futures::JsFuture {
        let dialog = self.dialog.clone();
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            dialog
                .add_event_listener_with_callback("close", &resolve)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise)
    }
}

/// Mount a `Dialog` under a `ModalHold` that counts takes and releases, as
/// the launcher provides one.
fn mount_modal(open: RwSignal<bool>) -> Modal {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, Ordering};
    let holds = Arc::new(AtomicI32::new(0));
    let hold = {
        let holds = holds.clone();
        ambre_chrome::ModalHold::new(move |held| {
            holds.fetch_add(if held { 1 } else { -1 }, Ordering::Relaxed);
        })
    };
    let heard = RwSignal::new(0);
    let on_close = Callback::new(move |()| heard.update(|n| *n += 1));
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let target: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let handle = mount_to(target, move || {
        provide_context(hold);
        view! {
            <ambre_chrome::Dialog open=open on_close=on_close>
                <button class="inside">"Inside"</button>
            </ambre_chrome::Dialog>
        }
        .into_any()
    });
    let dialog = root
        .query_selector("dialog")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    Modal {
        root,
        dialog,
        holds,
        heard,
        handle: Some(Box::new(handle)),
    }
}

/// A modal opened again before the `close` event of its last close lands
/// stays open (`CHROME-106`). The stale event clears no signal, releases no
/// hold and notifies no one.
#[wasm_bindgen_test]
async fn a_close_landing_after_a_reopen_leaves_the_modal_open() {
    let open = RwSignal::new(true);
    let modal = mount_modal(open);
    leptos::task::tick().await;
    assert_eq!(modal.holds(), 1);

    let closed = modal.closed();
    open.set(false);
    leptos::task::tick().await;
    assert!(!modal.dialog.open(), "closed, its event queued");
    open.set(true);
    leptos::task::tick().await;
    assert!(modal.dialog.open(), "open again before the event");
    closed.await.unwrap();
    assert!(modal.dialog.open(), "the stale close closed nothing");
    assert!(open.get_untracked(), "nor cleared the signal");
    assert_eq!(modal.holds(), 1, "nor released the loop");
    assert_eq!(modal.heard.get_untracked(), 0, "nor notified");
    modal.root.remove();
}

/// The native `close` event is queued. One that lands after the owner of
/// `on_close` is gone notifies no one, and panics nothing.
#[wasm_bindgen_test]
async fn a_close_after_its_callback_is_gone_notifies_no_one() {
    let open = RwSignal::new(true);
    let heard = RwSignal::new(0);
    let owner = Owner::new();
    let on_close = owner.with(|| Callback::new(move |()| heard.update(|n| *n += 1)));
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let target: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let handle = mount_to(target, move || {
        view! { <ambre_chrome::Dialog open=open on_close=on_close>"Body"</ambre_chrome::Dialog> }
            .into_any()
    });
    leptos::task::tick().await;
    let dialog: web_sys::HtmlDialogElement = root
        .query_selector("dialog")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert!(dialog.open());

    let modal = Modal {
        root,
        dialog,
        holds: std::sync::Arc::default(),
        heard,
        handle: Some(Box::new(handle)),
    };
    // A panic in the handler throws out of the event, to the window.
    let thrown = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = std::rc::Rc::clone(&thrown);
    let on_error = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        count.set(count.get() + 1);
    });
    let window = web_sys::window().unwrap();
    let listener = on_error.as_ref().unchecked_ref();
    window
        .add_event_listener_with_callback("error", listener)
        .unwrap();
    let closed = modal.closed();
    modal.dialog.close();
    owner.cleanup();
    closed.await.unwrap();
    window
        .remove_event_listener_with_callback("error", listener)
        .unwrap();
    assert_eq!(thrown.get(), 0, "the handler threw");
    assert_eq!(heard.get_untracked(), 0, "the callback was gone");
    assert!(!open.get_untracked(), "the signal still follows");
    modal.root.remove();
}

/// Setting the signal shows the modal (`CHROME-012`) and suspends the loop
/// (`CHROME-011`). Clearing it hides the modal (`CHROME-091`) and gives the
/// loop back (`CHROME-089`).
#[wasm_bindgen_test]
async fn a_modal_follows_its_signal_and_holds_the_loop_while_open() {
    let open = RwSignal::new(false);
    let modal = mount_modal(open);
    assert!(!modal.dialog.open());
    assert_eq!(modal.holds(), 0);

    open.set(true);
    leptos::task::tick().await;
    assert!(modal.dialog.open(), "the element is shown");
    assert_eq!(modal.holds(), 1, "the loop is held");

    let closed = modal.closed();
    open.set(false);
    closed.await.unwrap();
    assert!(!modal.dialog.open(), "the element is hidden");
    assert_eq!(modal.holds(), 0, "the loop is released");
    modal.root.remove();
}

/// A click on the backdrop closes the modal (`CHROME-088`) and clears its
/// signal (`CHROME-092`). A click inside it does neither.
#[wasm_bindgen_test]
async fn a_backdrop_click_dismisses_the_modal_and_clears_its_signal() {
    let open = RwSignal::new(true);
    let modal = mount_modal(open);
    leptos::task::tick().await;
    assert!(modal.dialog.open());

    let inside: web_sys::HtmlElement = modal
        .root
        .query_selector(".inside")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    inside.click();
    leptos::task::tick().await;
    sleep(10).await;
    assert!(open.get_untracked(), "a click inside keeps the modal");

    let closed = modal.closed();
    let click = web_sys::MouseEvent::new("click").unwrap();
    modal.dialog.dispatch_event(&click).unwrap();
    closed.await.unwrap();
    assert!(!modal.dialog.open(), "the backdrop closes it");
    assert!(!open.get_untracked(), "and clears the signal");
    assert_eq!(modal.holds(), 0);
    modal.root.remove();
}

/// A native close, the one Escape performs, clears the signal (`CHROME-088`,
/// `CHROME-092`), so the next open opens.
///
/// A headless browser drops a synthetic Escape, so the test closes the
/// element as the browser does, then reopens it through the signal.
#[wasm_bindgen_test]
async fn a_native_close_clears_the_signal_and_the_modal_reopens() {
    let open = RwSignal::new(true);
    let modal = mount_modal(open);
    leptos::task::tick().await;

    let closed = modal.closed();
    modal.dialog.close();
    closed.await.unwrap();
    assert!(!open.get_untracked(), "the signal follows the element");
    assert_eq!(modal.holds(), 0, "the loop is released");

    open.set(true);
    leptos::task::tick().await;
    assert!(modal.dialog.open(), "the next open is not blocked");
    assert_eq!(modal.holds(), 1);
    modal.root.remove();
}

/// An open modal keeps the focus inside it (`CHROME-028`) and leaves the page
/// behind it inert (`CHROME-068`). A control behind it cannot take the focus.
///
/// Closing it gives the page back, so the same control then takes the focus.
#[wasm_bindgen_test]
async fn an_open_modal_traps_the_focus_and_makes_the_page_inert() {
    let outside = document().create_element("button").unwrap();
    document().body().unwrap().append_child(&outside).unwrap();
    let outside: web_sys::HtmlElement = outside.dyn_into().unwrap();

    let open = RwSignal::new(true);
    let modal = mount_modal(open);
    leptos::task::tick().await;
    assert!(modal.dialog.matches(":modal").unwrap(), "opened as a modal");
    let inside = modal.root.query_selector(".inside").unwrap().unwrap();
    assert_eq!(active().as_ref(), Some(&inside), "the focus moved inside");

    outside.focus().unwrap();
    assert_eq!(active().as_ref(), Some(&inside), "the page behind is inert");

    let closed = modal.closed();
    open.set(false);
    closed.await.unwrap();
    outside.focus().unwrap();
    assert_eq!(
        active().as_ref(),
        Some(outside.as_ref()),
        "the page is back"
    );
    modal.root.remove();
    outside.remove();
}

/// A modal unmounted while open gives the loop back (`CHROME-089`). No
/// `close` event fires, so only the cleanup can release the hold.
#[wasm_bindgen_test]
async fn an_open_modal_unmounted_releases_the_loop() {
    let open = RwSignal::new(true);
    let mut modal = mount_modal(open);
    leptos::task::tick().await;
    assert_eq!(modal.holds(), 1);

    drop(modal.handle.take());
    assert_eq!(modal.holds(), 0, "the unmount released the hold");
    modal.root.remove();
}
