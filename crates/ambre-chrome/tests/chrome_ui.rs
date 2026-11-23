//! Reactive + DOM tests for the shared chrome toolkit (Epic `ET`,
//! `ambre-spec-testing.md`, "wasm" tier).
//!
//! The chrome components are leptos CSR built on `web-sys`/`wasm-bindgen`: the
//! reactive store and the mounted DOM only exist on the wasm target. So these run
//! under the `wasm-bindgen-test-runner` in headless Chrome (`just test-wasm`).
//!
//! Two belts (mirrors the trame chrome suite):
//!   - the **reactive belt** -- `CommandStates` over real leptos signals, no DOM;
//!   - the **DOM belt** -- components mounted into the real `document`, asserted
//!     through `web-sys` (ARIA roles, the `AppShell` canvas cell, toast text).
//!
//! On host the whole file is gated out (the `cfg` below), so `just test-native`
//! skips it. The pure decision logic (`Key::from_dom`, `place_menu`) is covered by
//! the `#[test]`s in `src/`.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_chrome::{
    AppShell, CommandStates, ContextMenu, FONT_SIZES, FontSizeSelect, FormatBar, FormatControl,
    FormatToggle, FormulaBar, Header, MenuEntry, Openness, PerfChip, PerfDetail, Pressed, RowShown,
    ShellRows, Sidebar, StatusBar, Tabs, ToastHost, ToastKind, ToastVoice, Toasts, Toolbar,
    ZoomChip,
};
use ambre_core::FontSize;
use ambre_core::{Categories, Commands, StaticKey};
use ambre_core::{Doc, Scale, View};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

/// The page point at `(x, y)`, in CSS px.
fn css_point(x: f32, y: f32) -> ambre_core::Point<ambre_core::Px<ambre_core::Css>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::Css>::new(x),
        ambre_core::Px::<ambre_core::Css>::new(y),
    )
}

wasm_bindgen_test_configure!(run_in_browser);

const A: StaticKey<Commands> = StaticKey::<Commands>::new("test.a");
const B: StaticKey<Commands> = StaticKey::<Commands>::new("test.b");

// -- Reactive belt: CommandStates (no DOM) -----------------------------------

#[wasm_bindgen_test]
fn command_states_defaults() {
    let owner = Owner::new();
    owner.set();
    let store = CommandStates::new([A, B]);
    // Toggles start `Off`, availability starts `Enabled` (actionable until
    // told else).
    assert_eq!(
        store.toggled_signal(A).unwrap().get_untracked(),
        ambre_chrome::Toggle::Off
    );
    assert_eq!(
        store.enabled_signal(A).unwrap().get_untracked(),
        ambre_chrome::Availability::Enabled
    );
}

#[wasm_bindgen_test]
fn command_states_single_writer_propagates() {
    let owner = Owner::new();
    owner.set();
    let store = CommandStates::new([A, B]);

    store.set_toggled(A, ambre_chrome::Toggle::On);
    store.set_enabled(B, ambre_chrome::Availability::Disabled);
    assert_eq!(
        store.toggled_signal(A).unwrap().get_untracked(),
        ambre_chrome::Toggle::On
    );
    assert_eq!(
        store.toggled_signal(B).unwrap().get_untracked(),
        ambre_chrome::Toggle::Off,
        "B toggle untouched"
    );
    assert_eq!(
        store.enabled_signal(B).unwrap().get_untracked(),
        ambre_chrome::Availability::Disabled
    );
    assert_eq!(
        store.enabled_signal(A).unwrap().get_untracked(),
        ambre_chrome::Availability::Enabled,
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
    store.set_toggled(B, ambre_chrome::Toggle::On);
    store.set_enabled(B, ambre_chrome::Availability::Disabled);
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

/// `CHROME-136`: a visual toast is hidden from assistive technology, so the
/// live region does not announce it, while an announced toast stays exposed.
#[wasm_bindgen_test]
fn a_visual_toast_is_hidden_from_a_screen_reader() {
    let owner = Owner::new();
    owner.set();
    let toasts = Toasts::new();
    toasts.raise(ToastKind::Error, ToastVoice::Visual, "refused");
    toasts.push(ToastKind::Error, "announced");
    let root = mount_in_container(move || view! { <ToastHost toasts=toasts/> });

    let shown: Vec<_> = (0..2)
        .map(|at| {
            let toast = root
                .query_selector_all(".ambre-toast")
                .unwrap()
                .get(at)
                .unwrap();
            let toast: web_sys::Element = toast.dyn_into().unwrap();
            (toast.text_content(), toast.get_attribute("aria-hidden"))
        })
        .collect();
    assert_eq!(
        shown,
        vec![
            (Some("refused".to_owned()), Some("true".to_owned())),
            (Some("announced".to_owned()), None),
        ]
    );

    root.remove();
}

#[wasm_bindgen_test]
fn app_shell_owns_the_named_canvas_cell() {
    let owner = Owner::new();
    owner.set();
    let root = mount_in_container(|| {
        view! {
            <AppShell canvas_id={ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT}>
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
    let zoom = RwSignal::new(Scale::<Doc, View>::within(1.25, 0.5, 2.0));
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom range=GRID /> });
    let btn = root.query_selector(".statusbar-zoom-btn").unwrap().unwrap();
    assert_eq!(btn.text_content().as_deref(), Some("125 %"));

    zoom.set(Scale::<Doc, View>::within(2.0, 0.5, 2.0));
    leptos::task::tick().await;
    assert_eq!(btn.text_content().as_deref(), Some("200 %"));

    root.remove();
}

/// The fps readout follows its signal after a change (`CHROME-086`).
#[wasm_bindgen_test]
async fn the_fps_readout_follows_its_signal() {
    let owner = Owner::new();
    owner.set();
    let fps = RwSignal::new(ambre_core::FrameRate::new(58.0));
    let root = mount_in_container(move || {
        view! { <PerfChip fps=Signal::derive(move || fps.get()) /> }
    });
    let btn = root.query_selector(".statusbar-fps").unwrap().unwrap();
    assert_eq!(btn.text_content().as_deref(), Some("58 fps"));

    fps.set(ambre_core::FrameRate::new(30.0));
    leptos::task::tick().await;
    assert_eq!(btn.text_content().as_deref(), Some("30 fps"));

    root.remove();
}

/// The range of a grid in these tests: the stops from 50 % to 200 %.
const GRID: ambre_core::ZoomRange<Doc> =
    ambre_core::ZoomRange::new(Scale::positive(0.5), Scale::positive(2.0));

/// `ZOOM-006`: the chip lists the stops of the one ladder inside the range
/// it is handed, high to low, and nothing else. A board and a grid read the
/// same stops where their ranges meet.
#[wasm_bindgen_test]
async fn the_zoom_chip_lists_the_one_ladder_inside_its_range() {
    let owner = Owner::new();
    owner.set();
    let listed = |root: &web_sys::Element| -> Vec<String> {
        let items = root.query_selector_all("[role=menuitemradio]").unwrap();
        (0..items.length())
            .filter_map(|i| items.item(i))
            .filter_map(|n| n.text_content())
            .collect()
    };
    let grid_zoom = RwSignal::new(Scale::<Doc, View>::ONE);
    let grid = mount_in_container(move || view! { <ZoomChip zoom=grid_zoom range=GRID /> });
    let board_zoom = RwSignal::new(Scale::<ambre_core::Board, View>::ONE);
    let board_range = ambre_core::ZoomRange::new(Scale::positive(0.1), Scale::positive(10.0));
    let board =
        mount_in_container(move || view! { <ZoomChip zoom=board_zoom range=board_range /> });
    for root in [&grid, &board] {
        let btn: web_sys::HtmlElement = root
            .query_selector(".statusbar-zoom-btn")
            .unwrap()
            .unwrap()
            .unchecked_into();
        btn.click();
    }
    leptos::task::tick().await;
    let ladder: Vec<String> = ambre_core::zoom_ladder::<Doc>()
        .into_iter()
        .rev()
        .map(|stop| format!("{:.0} %", stop.get() * 100.0))
        .collect();
    assert_eq!(listed(&board), ladder);
    assert_eq!(
        listed(&grid),
        ["200 %", "150 %", "125 %", "100 %", "75 %", "50 %"]
    );
    assert!(listed(&grid).iter().all(|stop| ladder.contains(stop)));
    grid.remove();
    board.remove();
}

#[wasm_bindgen_test]
fn zoom_chip_renders_readout_from_signal_and_starts_collapsed() {
    let owner = Owner::new();
    owner.set();
    // The readout mirrors the live factor (1.25 -> "125 %") and the preset menu
    // is collapsed on mount (only the button shows).
    let zoom = RwSignal::new(Scale::<Doc, View>::within(1.25, 0.5, 2.0));
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom range=GRID /> });

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
    let fps = RwSignal::new(ambre_core::FrameRate::new(58.0));
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
                pressed=Signal::derive(move || Pressed::from_bool(pressed.get()))
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
                pressed=Signal::derive(move || Pressed::from_bool(pressed.get()))
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
    let root = mount_in_container(
        move || view! { <FormatBar anchor=css_point(200.0, 100.0) controls=controls /> },
    );

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
    let zoom = RwSignal::new(Scale::<Doc, View>::ONE);
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom range=GRID /> });
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
    let zoom = RwSignal::new(Scale::<Doc, View>::ONE);
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom range=GRID /> });
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
    let zoom = RwSignal::new(Scale::<Doc, View>::ONE);
    let root = mount_in_container(move || view! { <ZoomChip zoom=zoom range=GRID /> });
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
    let pos = RwSignal::new(None::<ambre_core::Point<ambre_core::Px<ambre_core::Css>>>);
    let item = ambre_commands::Command {
        id: StaticKey::<Commands>::new("test.item"),
        title: |_| ambre_core::Validated::new("Item"),
        icon: None,
        keybindings: &[],
        category: StaticKey::<Categories>::new(""),
        home: ambre_commands::Home::ContextPanel,
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
    pos.set(Some(css_point(10.0, 10.0)));
    leptos::task::tick().await;
    let menu = root.query_selector(".ambre-ctx-menu").unwrap().unwrap();
    let inside = active().is_some_and(|el| menu.contains(Some(&el)));
    assert!(inside, "the menu took the focus");
    press(&menu, "Escape");
    leptos::task::tick().await;
    assert!(root.query_selector(".ambre-ctx-menu").unwrap().is_none());
    assert_eq!(active().as_ref(), opener.dyn_ref(), "the focus came back");

    pos.set(Some(css_point(10.0, 10.0)));
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
    let picked = RwSignal::new(Some(FontSize::from_whole_points(999)));
    let root = mount_in_container(move || {
        view! {
            <FontSizeSelect
                current=Signal::derive(|| Some(FontSize::from_whole_points(12)))
                on_pick=Callback::new(move |p: Option<FontSize>| picked.set(p))
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

fn palette_title(
    l: &ambre_commands::Locale,
) -> ambre_core::Validated<ambre_commands::CommandTitle> {
    let _ = l;
    ambre_core::Validated::new("Command palette")
}
fn hidden_title(l: &ambre_commands::Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> {
    let _ = l;
    ambre_core::Validated::new("Recalculate everything")
}
fn shown_title(l: &ambre_commands::Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> {
    let _ = l;
    ambre_core::Validated::new("Merge cells")
}

/// A table whose second command appears in no menu: exactly the case
/// `CHROME-008` is about.
const PALETTE_TABLE: &[ambre_commands::Command] = &[
    ambre_commands::Command {
        id: StaticKey::<Commands>::new("t.palette.open"),
        title: palette_title,
        icon: None,
        keybindings: &[ambre_commands::KeyBinding::primary(
            ambre_commands::KeyChar::K,
        )],
        category: StaticKey::<Categories>::new("general"),
        home: ambre_commands::Home::AmbreMenu,
    },
    ambre_commands::Command {
        id: StaticKey::<Commands>::new("t.engine.recalc"),
        title: hidden_title,
        icon: None,
        keybindings: &[],
        category: StaticKey::<Categories>::new("engine"),
        home: ambre_commands::Home::ContextPanel,
    },
    ambre_commands::Command {
        id: StaticKey::<Commands>::new("t.merge"),
        title: shown_title,
        icon: None,
        keybindings: &[],
        category: StaticKey::<Categories>::new("structure"),
        home: ambre_commands::Home::ContextPanel,
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
    let open = RwSignal::new(Openness::Open);
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

/// DOM test: each command row shows the name of its home region, and the chord
/// of a command that has one (`CHROME-157`, `CHROME-158`). A command with no
/// chord shows no chord chip.
#[wasm_bindgen_test]
fn palette_row_shows_home_region_and_chord() {
    let owner = Owner::new();
    owner.set();
    let open = RwSignal::new(Openness::Open);
    let root = mount_in_container(move || {
        view! {
            <ambre_chrome::CommandPalette
                open=open
                table=PALETTE_TABLE
                on_command=Callback::new(move |_| {})
            />
        }
    });

    let rows = root.query_selector_all(".ambre-palette-row").unwrap();
    assert_eq!(Ok(rows.length()), u32::try_from(PALETTE_TABLE.len()));
    for (i, command) in (0..rows.length()).zip(PALETTE_TABLE) {
        let row: web_sys::Element = rows.item(i).unwrap().dyn_into().unwrap();
        let home = row
            .query_selector(".ambre-palette-home")
            .unwrap()
            .and_then(|n| n.text_content());
        assert_eq!(
            home.as_deref(),
            Some(command.home.name(&ambre_commands::Locale::En).as_str()),
            "row {i} names its home region"
        );
        let chip = row.query_selector(".ambre-palette-kbd").unwrap();
        assert_eq!(
            chip.is_some(),
            command.chord().is_some(),
            "row {i} shows a chord chip exactly when the command has a chord"
        );
    }
    let chip = rows
        .item(0)
        .unwrap()
        .dyn_into::<web_sys::Element>()
        .unwrap()
        .query_selector(".ambre-palette-kbd")
        .unwrap()
        .and_then(|n| n.text_content())
        .unwrap_or_default();
    assert!(chip.contains('K'), "the chip reads the chord: {chip:?}");

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
    let open = RwSignal::new(Openness::Open);
    let invoked: RwSignal<Option<StaticKey<Commands>>> = RwSignal::new(None);
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

    assert_eq!(
        invoked.get_untracked(),
        Some(StaticKey::<Commands>::new("t.merge"))
    );
    assert_eq!(
        open.get_untracked(),
        Openness::Closed,
        "selecting a row closes the palette"
    );

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
    let open = RwSignal::new(Openness::Open);
    let chosen: RwSignal<Option<ambre_chrome::PaletteKey>> = RwSignal::new(None);
    let invoked: RwSignal<Option<StaticKey<Commands>>> = RwSignal::new(None);
    let key = ambre_chrome::PaletteKey::Channel(ambre_core::Id::from_bytes([7; 16]));
    let entries = Signal::stored(vec![ambre_chrome::PaletteEntry {
        key,
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

    assert_eq!(chosen.get_untracked(), Some(key));
    assert_eq!(invoked.get_untracked(), None);
    assert_eq!(
        open.get_untracked(),
        Openness::Closed,
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
fn mount_modal(open: RwSignal<Openness>) -> Modal {
    mount_modal_with(open, ambre_chrome::Dismissal::Free)
}

/// `mount_modal`, with what closes the modal besides its signal.
fn mount_modal_with(open: RwSignal<Openness>, dismissal: ambre_chrome::Dismissal) -> Modal {
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
            <ambre_chrome::Dialog open=open on_close=on_close dismissal=dismissal>
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
    let open = RwSignal::new(Openness::Open);
    let modal = mount_modal(open);
    leptos::task::tick().await;
    assert_eq!(modal.holds(), 1);

    let closed = modal.closed();
    open.set(Openness::Closed);
    leptos::task::tick().await;
    assert!(!modal.dialog.open(), "closed, its event queued");
    open.set(Openness::Open);
    leptos::task::tick().await;
    assert!(modal.dialog.open(), "open again before the event");
    closed.await.unwrap();
    assert!(modal.dialog.open(), "the stale close closed nothing");
    assert_eq!(
        open.get_untracked(),
        Openness::Open,
        "nor cleared the signal"
    );
    assert_eq!(modal.holds(), 1, "nor released the loop");
    assert_eq!(modal.heard.get_untracked(), 0, "nor notified");
    modal.root.remove();
}

/// The native `close` event is queued. One that lands after the owner of
/// `on_close` is gone notifies no one, and panics nothing.
#[wasm_bindgen_test]
async fn a_close_after_its_callback_is_gone_notifies_no_one() {
    let open = RwSignal::new(Openness::Open);
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
    assert_eq!(
        open.get_untracked(),
        Openness::Closed,
        "the signal still follows"
    );
    modal.root.remove();
}

/// Setting the signal shows the modal (`CHROME-012`) and suspends the loop
/// (`CHROME-011`). Clearing it hides the modal (`CHROME-091`) and gives the
/// loop back (`CHROME-089`).
#[wasm_bindgen_test]
async fn a_modal_follows_its_signal_and_holds_the_loop_while_open() {
    let open = RwSignal::new(Openness::Closed);
    let modal = mount_modal(open);
    assert!(!modal.dialog.open());
    assert_eq!(modal.holds(), 0);

    open.set(Openness::Open);
    leptos::task::tick().await;
    assert!(modal.dialog.open(), "the element is shown");
    assert_eq!(modal.holds(), 1, "the loop is held");

    let closed = modal.closed();
    open.set(Openness::Closed);
    closed.await.unwrap();
    assert!(!modal.dialog.open(), "the element is hidden");
    assert_eq!(modal.holds(), 0, "the loop is released");
    modal.root.remove();
}

/// A click on the backdrop closes the modal (`CHROME-088`) and clears its
/// signal (`CHROME-092`). A click inside it does neither.
#[wasm_bindgen_test]
async fn a_backdrop_click_dismisses_the_modal_and_clears_its_signal() {
    let open = RwSignal::new(Openness::Open);
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
    assert_eq!(
        open.get_untracked(),
        Openness::Open,
        "a click inside keeps the modal"
    );

    let closed = modal.closed();
    let click = web_sys::MouseEvent::new("click").unwrap();
    modal.dialog.dispatch_event(&click).unwrap();
    closed.await.unwrap();
    assert!(!modal.dialog.open(), "the backdrop closes it");
    assert_eq!(
        open.get_untracked(),
        Openness::Closed,
        "and clears the signal"
    );
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
    let open = RwSignal::new(Openness::Open);
    let modal = mount_modal(open);
    leptos::task::tick().await;

    let closed = modal.closed();
    modal.dialog.close();
    closed.await.unwrap();
    assert_eq!(
        open.get_untracked(),
        Openness::Closed,
        "the signal follows the element"
    );
    assert_eq!(modal.holds(), 0, "the loop is released");

    open.set(Openness::Open);
    leptos::task::tick().await;
    assert!(modal.dialog.open(), "the next open is not blocked");
    assert_eq!(modal.holds(), 1);
    modal.root.remove();
}

/// `CHROME-137`: a modal only its caller closes stays open on Escape and on
/// a backdrop click. Escape fires a `cancel` the modal prevents. A native
/// close it could not prevent shows it again. Its signal still closes it.
#[wasm_bindgen_test]
async fn an_explicit_modal_closes_on_its_signal_alone() {
    let open = RwSignal::new(Openness::Open);
    let modal = mount_modal_with(open, ambre_chrome::Dismissal::Explicit);
    leptos::task::tick().await;
    assert!(modal.dialog.open());

    // Any cancelable event named `cancel` reaches the handler.
    let init = web_sys::KeyboardEventInit::new();
    init.set_cancelable(true);
    let cancel =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("cancel", &init).unwrap();
    modal.dialog.dispatch_event(&cancel).unwrap();
    assert!(cancel.default_prevented(), "Escape was not refused");

    let click = web_sys::MouseEvent::new("click").unwrap();
    modal.dialog.dispatch_event(&click).unwrap();
    leptos::task::tick().await;
    sleep(10).await;
    assert_eq!(
        open.get_untracked(),
        Openness::Open,
        "the backdrop closed it"
    );

    let closed = modal.closed();
    modal.dialog.close();
    closed.await.unwrap();
    assert!(modal.dialog.open(), "a native close kept it closed");
    assert_eq!(open.get_untracked(), Openness::Open);
    assert_eq!(modal.holds(), 1, "the loop is held once");

    let closed = modal.closed();
    open.set(Openness::Closed);
    closed.await.unwrap();
    assert!(!modal.dialog.open(), "the signal did not close it");
    assert_eq!(modal.holds(), 0);
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

    let open = RwSignal::new(Openness::Open);
    let modal = mount_modal(open);
    leptos::task::tick().await;
    assert!(modal.dialog.matches(":modal").unwrap(), "opened as a modal");
    let inside = modal.root.query_selector(".inside").unwrap().unwrap();
    assert_eq!(active().as_ref(), Some(&inside), "the focus moved inside");

    outside.focus().unwrap();
    assert_eq!(active().as_ref(), Some(&inside), "the page behind is inert");

    let closed = modal.closed();
    open.set(Openness::Closed);
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
    let open = RwSignal::new(Openness::Open);
    let mut modal = mount_modal(open);
    leptos::task::tick().await;
    assert_eq!(modal.holds(), 1);

    drop(modal.handle.take());
    assert_eq!(modal.holds(), 0, "the unmount released the hold");
    modal.root.remove();
}

// -- The floating menu (context_menu.rs) -------------------------------------

/// A command of the floating menu's test table, labelled `title`.
fn ctx_command(
    id: &'static str,
    title: fn(&ambre_commands::Locale) -> ambre_core::Validated<ambre_commands::CommandTitle>,
) -> ambre_commands::Command {
    ambre_commands::Command {
        id: StaticKey::<Commands>::new(id),
        title,
        icon: None,
        keybindings: &[],
        category: StaticKey::<Categories>::new(""),
        home: ambre_commands::Home::ContextPanel,
    }
}

/// Five rows, a separator after the second: `Copy`, `Cut`, then `Paste`,
/// `Clear` and `Delete`. `Delete` is disabled.
fn ctx_entries() -> Vec<MenuEntry> {
    let en = ambre_commands::Locale::En;
    vec![
        MenuEntry::command(
            &ctx_command("t.copy", |_| ambre_core::Validated::new("Copy")),
            &en,
        ),
        MenuEntry::command(
            &ctx_command("t.cut", |_| ambre_core::Validated::new("Cut")),
            &en,
        ),
        MenuEntry::separator(),
        MenuEntry::command(
            &ctx_command("t.paste", |_| ambre_core::Validated::new("Paste")),
            &en,
        ),
        MenuEntry::command(
            &ctx_command("t.clear", |_| ambre_core::Validated::new("Clear")),
            &en,
        ),
        MenuEntry::command(
            &ctx_command("t.delete", |_| ambre_core::Validated::new("Delete")),
            &en,
        )
        .disabled(),
    ]
}

/// A floating menu mounted closed, the point that opens it, and every command
/// it ran, in order.
struct FloatingMenu {
    root: web_sys::Element,
    pos: RwSignal<Option<ambre_core::Point<ambre_core::Px<ambre_core::Css>>>>,
    ran: RwSignal<Vec<StaticKey<Commands>>>,
}

impl FloatingMenu {
    /// Mounts the menu over [`ctx_entries`], beside a button outside it.
    fn mount() -> Self {
        let pos = RwSignal::new(None);
        let ran = RwSignal::new(Vec::new());
        let on_command = Callback::new(move |id| ran.update(|r| r.push(id)));
        let root = mount_in_container(move || {
            view! {
                <button class="test-outside">"Outside"</button>
                <ContextMenu pos=pos entries=ctx_entries() on_command=on_command/>
            }
        });
        Self { root, pos, ran }
    }

    /// Opens the menu and waits for its render.
    async fn open(&self) -> web_sys::Element {
        self.pos.set(Some(css_point(20.0, 20.0)));
        leptos::task::tick().await;
        self.menu().unwrap()
    }

    fn menu(&self) -> Option<web_sys::Element> {
        self.root.query_selector(".ambre-ctx-menu").unwrap()
    }

    /// The label of the row that holds the focus.
    fn focused(&self) -> Option<String> {
        let el = active()?;
        el.matches("[role=menuitem]").ok()?.then_some(())?;
        el.query_selector(".ambre-ctx-label").ok()??.text_content()
    }
}

/// Presses `key` on the menu and waits for the focus to follow.
async fn press_menu(menu: &web_sys::Element, key: &str) {
    press(menu, key);
    leptos::task::tick().await;
}

/// Dispatch a bubbling primary `pointerdown` on `target`, as a press there
/// raises.
fn pointer_down(target: &web_sys::EventTarget) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_is_primary(true);
    let ev = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// The floating menu is operable from the keyboard alone (`CHROME-026`). The
/// arrows move, wrap and skip the separator. Home and End jump to the ends. A
/// letter jumps to the next row it starts, and Enter runs the focused row.
#[wasm_bindgen_test]
async fn the_floating_menu_is_operable_by_keyboard_alone() {
    let owner = Owner::new();
    owner.set();
    let m = FloatingMenu::mount();
    let menu = m.open().await;
    assert_eq!(
        m.focused().as_deref(),
        Some("Copy"),
        "the menu took the focus"
    );

    press_menu(&menu, "ArrowDown").await;
    assert_eq!(m.focused().as_deref(), Some("Cut"));
    press_menu(&menu, "ArrowDown").await;
    assert_eq!(
        m.focused().as_deref(),
        Some("Paste"),
        "the separator is skipped"
    );
    press_menu(&menu, "End").await;
    assert_eq!(m.focused().as_deref(), Some("Delete"));
    press_menu(&menu, "ArrowDown").await;
    assert_eq!(m.focused().as_deref(), Some("Copy"), "ArrowDown wrapped");
    press_menu(&menu, "ArrowUp").await;
    assert_eq!(m.focused().as_deref(), Some("Delete"), "ArrowUp wrapped");
    press_menu(&menu, "Home").await;
    assert_eq!(m.focused().as_deref(), Some("Copy"));

    press_menu(&menu, "p").await;
    assert_eq!(m.focused().as_deref(), Some("Paste"), "a letter jumps");
    press_menu(&menu, "c").await;
    assert_eq!(
        m.focused().as_deref(),
        Some("Clear"),
        "to the next row it starts"
    );
    press_menu(&menu, "c").await;
    assert_eq!(m.focused().as_deref(), Some("Copy"), "and wraps");
    press_menu(&menu, "x").await;
    assert_eq!(
        m.focused().as_deref(),
        Some("Copy"),
        "a letter no row starts moves nothing"
    );
    assert!(m.ran.get_untracked().is_empty(), "moving runs nothing");

    press_menu(&menu, "c").await;
    assert_eq!(m.focused().as_deref(), Some("Cut"));
    press_menu(&menu, "Enter").await;
    assert_eq!(
        m.ran.get_untracked(),
        vec![StaticKey::<Commands>::new("t.cut")],
        "Enter ran the focused row"
    );
    assert!(m.menu().is_none(), "and closed the menu");
    assert_eq!(m.pos.get_untracked(), None, "and cleared its point");
    m.root.remove();
}

/// A disabled row runs nothing, neither from the keyboard nor from a click,
/// and the menu stays open (`CHROME-112`).
#[wasm_bindgen_test]
async fn a_disabled_row_runs_nothing_from_the_keyboard_or_the_pointer() {
    let owner = Owner::new();
    owner.set();
    let m = FloatingMenu::mount();
    let menu = m.open().await;
    press_menu(&menu, "End").await;
    assert_eq!(m.focused().as_deref(), Some("Delete"));
    let row: web_sys::HtmlElement = active().unwrap().dyn_into().unwrap();
    assert_eq!(row.get_attribute("aria-disabled").as_deref(), Some("true"));

    press_menu(&menu, "Enter").await;
    press_menu(&menu, " ").await;
    row.click();
    leptos::task::tick().await;
    assert!(m.ran.get_untracked().is_empty(), "the disabled row ran");
    assert!(m.menu().is_some(), "the menu stays open");
    m.pos.set(None);
    leptos::task::tick().await;
    m.root.remove();
}

/// A press outside the open menu closes it and runs nothing. A press inside
/// it leaves it open (`CHROME-111`).
#[wasm_bindgen_test]
async fn a_press_outside_closes_the_floating_menu_and_one_inside_does_not() {
    let owner = Owner::new();
    owner.set();
    let m = FloatingMenu::mount();
    let menu = m.open().await;

    let row = menu.query_selector("[data-idx=\"2\"]").unwrap().unwrap();
    pointer_down(&row);
    leptos::task::tick().await;
    assert!(m.menu().is_some(), "a press inside keeps the menu");

    let outside = m.root.query_selector(".test-outside").unwrap().unwrap();
    pointer_down(&outside);
    leptos::task::tick().await;
    assert!(m.menu().is_none(), "a press outside closes the menu");
    assert_eq!(m.pos.get_untracked(), None);
    assert!(m.ran.get_untracked().is_empty(), "and runs nothing");
    m.root.remove();
}

/// A press anywhere once the floating menu is unmounted throws nothing
/// (`CHROME-113`). The press listener lives on the document, past the menu,
/// and a Toile menu bar unmounts its menu on each presentation.
#[wasm_bindgen_test]
async fn a_press_after_the_floating_menu_is_unmounted_throws_nothing() {
    let child = Owner::new();
    let root = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&root).unwrap();
    let target: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
    let handle = child.with(|| {
        let pos = RwSignal::new(Some(css_point(10.0, 10.0)));
        mount_to(target, move || {
            view! { <ContextMenu pos=pos entries=ctx_entries() on_command=Callback::new(|_| {})/> }
        })
    });
    leptos::task::tick().await;
    assert!(root.query_selector(".ambre-ctx-menu").unwrap().is_some());
    // What a `Show` does when it hides its children: unmount, then dispose.
    drop(handle);
    child.cleanup();
    root.remove();

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
    pointer_down(&document().body().unwrap());
    window
        .remove_event_listener_with_callback("error", listener)
        .unwrap();
    assert_eq!(thrown.get(), 0, "the press threw");
}

// -- The command palette, driven by keys (palette.rs) -------------------------

/// The palette is operable from the keyboard alone (`CHROME-114`). The arrows
/// move the highlight and wrap. Enter runs the highlighted row, and Escape
/// closes the palette with nothing run.
#[wasm_bindgen_test]
async fn the_palette_is_operable_by_keyboard_alone() {
    let owner = Owner::new();
    owner.set();
    let open = RwSignal::new(Openness::Open);
    let ran = RwSignal::new(Vec::new());
    let root = mount_in_container(move || {
        view! {
            <ambre_chrome::CommandPalette
                open=open
                table=PALETTE_TABLE
                on_command=Callback::new(move |id| ran.update(|r: &mut Vec<_>| r.push(id)))
            />
        }
    });
    leptos::task::tick().await;
    let query = root
        .query_selector(".ambre-palette-query")
        .unwrap()
        .unwrap();
    let highlighted = || {
        root.query_selector(".ambre-palette-row[aria-selected=true] .ambre-palette-label")
            .unwrap()
            .and_then(|e| e.text_content())
    };
    assert_eq!(highlighted().as_deref(), Some("Command palette"));

    press(&query, "ArrowUp");
    leptos::task::tick().await;
    assert_eq!(
        highlighted().as_deref(),
        Some("Merge cells"),
        "ArrowUp wrapped"
    );
    press(&query, "ArrowDown");
    press(&query, "ArrowDown");
    leptos::task::tick().await;
    assert_eq!(
        highlighted().as_deref(),
        Some("Recalculate everything"),
        "ArrowDown wrapped"
    );
    press(&query, "Enter");
    leptos::task::tick().await;
    assert_eq!(
        ran.get_untracked(),
        vec![StaticKey::<Commands>::new("t.engine.recalc")],
        "Enter ran the highlighted row"
    );
    assert_eq!(
        open.get_untracked(),
        Openness::Closed,
        "and closed the palette"
    );

    open.set(Openness::Open);
    leptos::task::tick().await;
    press(&query, "Escape");
    leptos::task::tick().await;
    assert_eq!(open.get_untracked(), Openness::Closed, "Escape closed it");
    assert_eq!(ran.get_untracked().len(), 1, "Escape ran nothing");
    // The native close lands on a later task: wait for it while the modal lives.
    sleep(10).await;
    root.remove();
}

/// Each open starts from a blank query on the first row (`CHROME-115`), so
/// the last search never picks the row the next Enter runs.
#[wasm_bindgen_test]
async fn a_reopened_palette_starts_blank_on_the_first_row() {
    let owner = Owner::new();
    owner.set();
    let open = RwSignal::new(Openness::Open);
    let root = mount_in_container(move || {
        view! {
            <ambre_chrome::CommandPalette open=open table=PALETTE_TABLE on_command=Callback::new(|_| {})/>
        }
    });
    leptos::task::tick().await;
    let query: web_sys::HtmlInputElement = root
        .query_selector(".ambre-palette-query")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    query.set_value("merge");
    query
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    press(&query, "ArrowDown");
    leptos::task::tick().await;
    assert_eq!(palette_labels(&root), vec!["Merge cells".to_owned()]);

    open.set(Openness::Closed);
    sleep(10).await;
    open.set(Openness::Open);
    sleep(10).await;
    assert_eq!(query.value(), "", "the query is blank");
    assert_eq!(
        palette_labels(&root).len(),
        PALETTE_TABLE.len(),
        "every row is back"
    );
    let first = root.query_selector(".ambre-palette-row").unwrap().unwrap();
    assert_eq!(
        first.get_attribute("aria-selected").as_deref(),
        Some("true")
    );
    assert_eq!(
        active().as_ref(),
        Some(query.as_ref()),
        "the query holds the caret"
    );
    open.set(Openness::Closed);
    sleep(10).await;
    root.remove();
}

// -- The shell slots (app_shell.rs) ------------------------------------------

/// The shell puts the toolbar and the sidebar an application hands it in
/// their regions. The sidebar folds from its own control and stays in the
/// page, its name kept.
#[wasm_bindgen_test]
async fn the_shell_fills_its_toolbar_and_sidebar_and_the_sidebar_folds() {
    let owner = Owner::new();
    owner.set();
    let root = mount_in_container(|| {
        view! {
            <AppShell>
                <Header slot>"H"</Header>
                <Toolbar slot><button class="test-tool">"Bold"</button></Toolbar>
                <Sidebar slot label="Channels"><p class="test-nav">"General"</p></Sidebar>
            </AppShell>
        }
    });
    assert!(
        root.query_selector(".ambre-commandbar .test-tool")
            .unwrap()
            .is_some()
    );
    let nav = root.query_selector("nav.ambre-sidebar").unwrap().unwrap();
    assert_eq!(nav.get_attribute("aria-label").as_deref(), Some("Channels"));
    assert!(nav.query_selector(".test-nav").unwrap().is_some());
    let fold: web_sys::HtmlElement = nav
        .query_selector(".ambre-sidebar-fold")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(fold.get_attribute("aria-expanded").as_deref(), Some("true"));

    fold.click();
    leptos::task::tick().await;
    assert_eq!(
        fold.get_attribute("aria-expanded").as_deref(),
        Some("false")
    );
    assert_eq!(nav.get_attribute("data-expanded").as_deref(), Some("false"));
    assert!(nav.is_connected(), "a folded sidebar stays in the page");

    fold.click();
    leptos::task::tick().await;
    assert_eq!(nav.get_attribute("data-expanded").as_deref(), Some("true"));
    root.remove();
}

/// A row the application marks absent renders no element, though its slot is
/// handed (`CHROME-199`). A shown row keeps its content.
#[wasm_bindgen_test]
fn an_absent_row_renders_no_element_and_a_shown_row_keeps_its_content() {
    let owner = Owner::new();
    owner.set();
    let rows = ShellRows {
        toolbar: RowShown::Shown,
        formula_bar: RowShown::Absent,
        tabs: RowShown::Absent,
    };
    let root = mount_in_container(move || {
        view! {
            <AppShell rows=rows>
                <Header slot>"H"</Header>
                <Toolbar slot><button class="test-tool">"Menu"</button></Toolbar>
                <FormulaBar slot><input class="test-formula" /></FormulaBar>
                <Tabs slot><button class="test-tab">"Sheet1"</button></Tabs>
            </AppShell>
        }
    });
    let present = |selector: &str| root.query_selector(selector).unwrap().is_some();
    assert!(present(".ambre-commandbar .test-tool"));
    assert!(!present(".ambre-formulabar"), "an absent formula bar");
    assert!(!present(".test-formula"));
    assert!(!present(".ambre-tabstrip"), "an absent tab strip");
    assert!(!present(".test-tab"));
    root.remove();

    let none = ShellRows {
        toolbar: RowShown::Absent,
        ..rows
    };
    let root = mount_in_container(move || {
        view! {
            <AppShell rows=none>
                <Header slot>"H"</Header>
                <Toolbar slot><button class="test-tool">"Menu"</button></Toolbar>
            </AppShell>
        }
    });
    assert!(
        root.query_selector(".ambre-commandbar").unwrap().is_none(),
        "an absent command bar"
    );
    root.remove();
}
