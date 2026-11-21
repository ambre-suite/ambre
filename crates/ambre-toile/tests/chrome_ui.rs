//! DOM tests for the Toile chrome (Epic `ET`, `ambre-spec-architecture.md`
//! "wasm" tier) -- the render and input belt of `toile-board-rendering`.
//!
//! The chrome (`ambre_toile::chrome`) is `cfg(target_arch = "wasm32")` -- leptos
//! CSR + DOM (`web-sys`/`wasm-bindgen`) that does not compile on the host. So its
//! tests run under the wasm target via the `wasm-bindgen-test-runner`, in
//! headless Chrome (`just test-wasm`).
//!
//! The pure models are host-tested in `ui::{status,menu,menu_bar,palette}`; this
//! belt mounts the real components and asserts the composed DOM plus input:
//! - the selection status band -- aggregates + shared zoom/fps chips;
//! - the right-click context menu end-to-end -- the entry rows a
//!   [`CtxScope`] renders, and that clicking a row fires its `StaticKey<Commands>`;
//! - the tool palette -- a click switches the shared active tool;
//! - the top menu bar and the floating selection toolbar -- a click fires
//!   its `StaticKey<Commands>` on the shared sink.
//!
//! On the host target the whole file is empty (the `cfg` gate below), so the
//! native tier (`just test-native`) skips it cleanly.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::UiEvent;
use ambre_bus::interop::{
    Direction, ExportedFile, FileFormat, InteropAnswer, InteropError, Occurrences, Outcome,
};
use ambre_canvas::PerfSnapshot;
use ambre_chrome::CommandStates;
use ambre_chrome::ContextMenu;
use ambre_chrome::{ToastHost, Toasts};
use ambre_core::id::{Document, Frame, Id, Thread, User};
use ambre_core::index::{Len, Pos};
use ambre_core::{Authored, Char, Count, Instant};
use ambre_core::{Board, Scale, View};
use ambre_core::{Commands, StaticKey};
use ambre_draw::Rect;
use ambre_toile::chrome::{
    CommentComposer, CommentDraft, CommentIntent, CommentsPanel, DeckIntent, DeckMove, DeckPanel,
    EditingSurfaces, FloatBar, PresenterIntent, PresenterPanel, REDO, SelStats, SelectionToolbar,
    StatusBar, StyleEdit, StyleModel, Toggles, ToileIntent, ToileRelay, ToolPalette, TopMenuBar,
    UNDO,
};
use ambre_toile::message::TextDelta;
use ambre_toile::message::{
    Broadcast, CommentAnchor, CommentMessage, CommentSummary, CommentThread, DeckFrame, DeckView,
    Elements, PresencePayload, PresenceUpdate, PresentMode, Slide, SlidePosition, ToileCommand,
    ToileEvent,
};
use ambre_toile::message::{ToileConstruct, ToileReport};
use ambre_toile::ui::menu::{self, ClickSite, CtxScope, GroupHeld, LockMix};
use ambre_toile::ui::menu_bar::ZOOM_IN;
use ambre_toile::ui::{PanelState, Tool};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

/// The events the chrome reads in these tests: ToileEvent documents, no channel.
type Event = UiEvent<ToileEvent>;

/// The page point at `(x, y)`, in CSS px.
fn css_point(x: f32, y: f32) -> ambre_core::Point<ambre_core::Px<ambre_core::Css>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::Css>::new(x),
        ambre_core::Px::<ambre_core::Css>::new(y),
    )
}

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `f` into a detached container appended to the body, returning it so the
/// test can query and then drop it. The mount lives as long as the test's
/// owner. An effect it queued then dies with the test's signals, rather than
/// reading a disposed one when a later test flushes the queue.
fn mount_in_container<F, N>(f: F) -> web_sys::Element
where
    F: FnOnce() -> N + 'static,
    N: IntoView + 'static,
{
    let container = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&container).unwrap();
    let target: web_sys::HtmlElement = container.clone().dyn_into().unwrap();
    StoredValue::new_local(mount_to(target, f));
    container
}

fn mount_bar(
    stats: Option<SelStats>,
    zoom: f64,
    fps: ambre_core::FrameRate,
) -> (web_sys::Element, RwSignal<Scale<Board, View>>) {
    let stats = RwSignal::new(stats);
    let zoom = RwSignal::new(Scale::<Board, View>::within(zoom, 0.1, 10.0));
    let perf = RwSignal::new(PerfSnapshot {
        fps,
        ..PerfSnapshot::default()
    });
    let root = mount_in_container(move || view! { <StatusBar stats=stats perf=perf zoom=zoom /> });
    (root, zoom)
}

/// `CHROME-206`: the zoom chip closes the status bar, at its right end.
#[wasm_bindgen_test]
fn the_zoom_chip_closes_the_status_bar() {
    let owner = Owner::new();
    owner.set();
    let (root, _zoom) = mount_bar(None, 1.0, ambre_core::FrameRate::new(60.0));
    let last = root.last_element_child().expect("a status bar child");
    assert!(
        last.class_list().contains("statusbar-zoom"),
        "the band ends on `{}`",
        last.class_name()
    );
    root.remove();
}

/// `TOILE-UI-053`: the mounted band shows the count of a multi-selection.
#[wasm_bindgen_test]
fn multi_selection_shows_count_bounds_and_chips() {
    let owner = Owner::new();
    owner.set();
    let stats = SelStats {
        count: ambre_core::Count::<ambre_toile::message::Elements>::new(3),
        bounds: Some(Rect::new(10.0, 20.0, 50.0, 80.0)),
    };
    let (root, _zoom) = mount_bar(Some(stats), 1.5, ambre_core::FrameRate::new(60.0));

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

/// `TOILE-UI-107`: an empty selection leaves the mounted band empty.
#[wasm_bindgen_test]
fn empty_selection_shows_only_the_chips() {
    let owner = Owner::new();
    owner.set();
    // No selection reply yet: the aggregates area is empty, but the zoom chip
    // (and fps chip) still render.
    let (root, _zoom) = mount_bar(None, 1.0, ambre_core::FrameRate::new(30.0));

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

/// `TOILE-UI-106`: a lone element shows its count and no bounds in the mounted band.
#[wasm_bindgen_test]
fn single_selection_shows_count_without_bounds() {
    let owner = Owner::new();
    owner.set();
    // A lone element: the count shows, but no combined-bounds caption.
    let stats = SelStats {
        count: ambre_core::Count::<ambre_toile::message::Elements>::new(1),
        bounds: Some(Rect::new(0.0, 0.0, 10.0, 10.0)),
    };
    let (root, _zoom) = mount_bar(Some(stats), 1.0, ambre_core::FrameRate::new(60.0));

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

// -- Context menu -----------------------------------------------------
// The pure `CtxScope -> entries` mapping and the `StaticKey<Commands> -> ToileCommand`
// resolver are host-tested in `ui::menu`; here we mount the real
// `ambre_chrome::ContextMenu` fed by `chrome::context_entries_for` and assert
// the DOM rows a scope renders, plus that clicking a row fires its `StaticKey<Commands>`
// on the shared `on_command` sink (the exact bridge the shell wires, `app_root`).

/// Mount a `ContextMenu` opened at a fixed point, fed by the entries a `scope`
/// renders. Returns the container and the signal the menu's `on_command` writes
/// (so a test can click a row and read back the fired `StaticKey<Commands>`).
fn mount_ctx_menu(scope: CtxScope) -> (web_sys::Element, RwSignal<Option<StaticKey<Commands>>>) {
    let entries = RwSignal::new(ambre_toile::chrome::context_entries_for(&scope));
    let pos = RwSignal::new(Some(css_point(10.0, 10.0)));
    let fired = RwSignal::new(None::<StaticKey<Commands>>);
    let on_command = Callback::new(move |id: StaticKey<Commands>| fired.set(Some(id)));
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

/// `TOILE-UI-047`: the empty-canvas menu shows only the Paste and Select all rows.
#[wasm_bindgen_test]
fn context_menu_empty_canvas_shows_only_paste_and_select_all() {
    let owner = Owner::new();
    owner.set();
    // The empty-canvas scope offers exactly Paste + Select all.
    let (root, fired) = mount_ctx_menu(CtxScope::default());
    assert_eq!(ctx_item_labels(&root), vec!["Paste", "Select all"]);

    assert!(click_ctx_item(&root, "Select all"));
    assert_eq!(fired.get_untracked(), Some(menu::SELECT_ALL));

    root.remove();
}

/// `TOILE-UI-047`: a single unlocked element shows no Group row and no Unlock row.
#[wasm_bindgen_test]
fn context_menu_copy_row_fires_copy_command() {
    let owner = Owner::new();
    owner.set();
    // A single unlocked element: the clipboard group renders; clicking Copy fires
    // the COPY id the view resolves against its selection.
    let scope = CtxScope {
        site: ClickSite::Selection,
        count: Count::<Elements>::new(1),
        locks: LockMix::Unlocked,
        ..CtxScope::default()
    };
    let (root, fired) = mount_ctx_menu(scope);
    let labels = ctx_item_labels(&root);
    assert!(labels.contains(&"Copy".to_owned()), "labels: {labels:?}");
    assert!(labels.contains(&"Delete".to_owned()), "labels: {labels:?}");
    // A single-element, unlocked, group-less selection offers neither grouping
    // side nor Unlock (the omission-when-inapplicable rule of).
    assert!(!labels.contains(&"Group".to_owned()), "labels: {labels:?}");
    assert!(!labels.contains(&"Unlock".to_owned()), "labels: {labels:?}");

    assert!(click_ctx_item(&root, "Copy"));
    assert_eq!(fired.get_untracked(), Some(menu::COPY));

    root.remove();
}

/// `TOILE-UI-415`: choosing Group in the context menu reports the group
/// command id.
#[wasm_bindgen_test]
fn context_menu_multi_selection_offers_group() {
    let owner = Owner::new();
    owner.set();
    let scope = CtxScope {
        site: ClickSite::Selection,
        count: Count::<Elements>::new(2),
        locks: LockMix::Unlocked,
        ..CtxScope::default()
    };
    let (root, fired) = mount_ctx_menu(scope);
    assert!(ctx_item_labels(&root).contains(&"Group".to_owned()));

    assert!(click_ctx_item(&root, "Group"));
    assert_eq!(fired.get_untracked(), Some(menu::GROUP));

    root.remove();
}

/// `TOILE-UI-047`: a locked selection shows the Unlock row and no Lock row.
#[wasm_bindgen_test]
fn context_menu_locked_selection_offers_only_unlock() {
    let owner = Owner::new();
    owner.set();
    // A wholly-locked selection: Unlock applies, Lock is omitted.
    let scope = CtxScope {
        site: ClickSite::Selection,
        count: Count::<Elements>::new(1),
        locks: LockMix::Locked,
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

// -- Tool palette, input --------------------------------------

/// `TOILE-UI-408`: a palette button arms the tool it names.
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

// -- Top menu bar, zoom from the View menu -----------------------

/// `TOILE-UI-416`: the zoom-in entry of the View menu reports the zoom-in
/// command. The bar itself holds no zoom control: the zoom lives in the status
/// bar alone (`CHROME-206`).
#[wasm_bindgen_test]
async fn the_view_menu_reports_zoom_in_and_the_bar_holds_no_zoom_control() {
    let owner = Owner::new();
    owner.set();
    let fired = RwSignal::new(None::<StaticKey<Commands>>);
    let on_command = Callback::new(move |id: StaticKey<Commands>| fired.set(Some(id)));
    let store = CommandStates::new(ambre_toile::chrome::COMMANDS.iter().map(|c| c.id));
    ambre_toile::chrome::init_command_states(&store);
    let toggles = Toggles::from_store(&store);
    let root = mount_in_container(move || {
        view! { <TopMenuBar on_command=on_command toggles=toggles /> }
    });
    assert!(
        root.query_selector("[aria-label=\"Zoom in\"]")
            .unwrap()
            .is_none(),
        "the menu bar holds a zoom control"
    );
    assert!(root.query_selector(".ambre-toile-zoom").unwrap().is_none());

    assert!(click_by_aria(&root, "View"));
    leptos::task::tick().await;
    let rows = root.query_selector_all(".ambre-ctx-item").unwrap();
    let zoom_in = (0..rows.length())
        .filter_map(|i| rows.item(i))
        .find(|row| row.text_content().is_some_and(|t| t.starts_with("Zoom in")))
        .expect("a Zoom in row in the View menu");
    zoom_in.dyn_into::<web_sys::HtmlElement>().unwrap().click();
    assert_eq!(fired.get_untracked(), Some(ZOOM_IN));

    root.remove();
}

/// `mirror_event` projects the authoritative Undo/Redo availability from
/// `ToileEvent::UndoState` into the command store the menu bar reads for its
/// `disabled` state (`UNDO-006`).
#[wasm_bindgen_test]
fn undo_state_mirrors_into_command_store() {
    let owner = Owner::new();
    owner.set();

    let store = CommandStates::new(ambre_toile::chrome::COMMANDS.iter().map(|c| c.id));
    ambre_toile::chrome::init_command_states(&store);
    // A fresh board starts with an empty history.
    assert_eq!(
        store.enabled_signal(UNDO).unwrap().get(),
        ambre_chrome::Availability::Disabled
    );
    assert_eq!(
        store.enabled_signal(REDO).unwrap().get(),
        ambre_chrome::Availability::Disabled
    );

    let ev = Event::Viewport {
        document_id: Id::<Document>::mint(),
        event: ToileEvent::UndoState {
            undo: ambre_core::Availability::Enabled,
            redo: ambre_core::Availability::Disabled,
        },
    };
    ambre_toile::chrome::mirror_event(&store, &ev);
    assert_eq!(
        store.enabled_signal(UNDO).unwrap().get(),
        ambre_chrome::Availability::Enabled
    );
    assert_eq!(
        store.enabled_signal(REDO).unwrap().get(),
        ambre_chrome::Availability::Disabled
    );
}

// -- Floating selection toolbar, input ------------------------

/// `TOILE-UI-419`: the floating bar's Delete button reports the command id
/// the context menu's Delete entry carries.
#[wasm_bindgen_test]
fn selection_toolbar_delete_fires_command() {
    let owner = Owner::new();
    owner.set();
    // A live single-element bar: Delete is always present; Group/Ungroup/Lock are
    // gated by the same flags as the context menu.
    let bar = RwSignal::new(Some(FloatBar {
        at: css_point(40.0, 20.0),
        count: Count::<Elements>::new(1),
        groups: GroupHeld::NoGroup,
        locks: LockMix::Unlocked,
        fill: None,
        mixed: Default::default(),
    }));
    let fired = RwSignal::new(None::<StaticKey<Commands>>);
    let on_command = Callback::new(move |id: StaticKey<Commands>| fired.set(Some(id)));
    let model = RwSignal::new(None::<StyleModel>);
    let on_style = Callback::new(move |_e: StyleEdit| {});
    let root = mount_in_container(move || {
        view! { <SelectionToolbar bar=bar model=model on_command=on_command on_style=on_style /> }
    });

    assert!(click_by_aria(&root, "Delete"));
    assert_eq!(fired.get_untracked(), Some(menu::DELETE));

    root.remove();
}

/// Assert that `root` holds a control announcing the registry title of each
/// of `ids`.
fn assert_registry_labels(root: &web_sys::Element, ids: &[StaticKey<Commands>]) {
    for &id in ids {
        let expected = ambre_toile::ui::commands::title(id, &ambre_commands::Locale::En)
            .expect("the command is described");
        let found = root
            .query_selector(&format!("[aria-label=\"{expected}\"]"))
            .unwrap();
        assert!(found.is_some(), "{id:?}: no control announces {expected:?}");
    }
}

/// The tool palette and the selection toolbar announce the title each
/// command's description carries (`UI-THREAD-036`).
#[wasm_bindgen_test]
fn the_button_bars_announce_the_registry_labels() {
    use ambre_toile::ui::palette::{
        TOOL_COMMENT, TOOL_CONNECTOR, TOOL_DRAW, TOOL_HAND, TOOL_LASER, TOOL_PEN, TOOL_SELECT,
        TOOL_SHAPE, TOOL_STICKY, TOOL_TEXT,
    };
    let owner = Owner::new();
    owner.set();

    let active = RwSignal::new(Tool::Select);
    let palette = mount_in_container(move || view! { <ToolPalette active=active /> });
    assert_registry_labels(
        &palette,
        &[
            TOOL_SELECT,
            TOOL_PEN,
            TOOL_STICKY,
            TOOL_SHAPE,
            TOOL_CONNECTOR,
            TOOL_DRAW,
            TOOL_TEXT,
            TOOL_COMMENT,
            TOOL_LASER,
            TOOL_HAND,
        ],
    );
    palette.remove();

    let on_command = Callback::new(|_: StaticKey<Commands>| {});
    // A grouped selection holding a locked and an unlocked element shows every
    // command button the bar has.
    let bar = RwSignal::new(Some(FloatBar {
        at: css_point(40.0, 20.0),
        count: Count::<Elements>::new(2),
        groups: GroupHeld::Group,
        locks: LockMix::Both,
        fill: None,
        mixed: Default::default(),
    }));
    let model = RwSignal::new(None::<StyleModel>);
    let on_style = Callback::new(move |_e: StyleEdit| {});
    let toolbar = mount_in_container(move || {
        view! { <SelectionToolbar bar=bar model=model on_command=on_command on_style=on_style /> }
    });
    assert_registry_labels(
        &toolbar,
        &[menu::DUPLICATE, menu::DELETE, menu::LOCK, menu::UNLOCK],
    );
    toolbar.remove();
}

// -- Presenter overlay panel (toile-presentation), input ---------------------

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

/// Mount a `PresenterPanel` with full "Follow presenter" state. That is the
/// broadcast toggle, the active follow target, and the latest broadcasting peer
/// (which drives whether the Follow button renders).
fn mount_presenter_follow(
    open: bool,
    index: usize,
    count: usize,
    notes: &str,
    broadcasting: bool,
    following: Option<Id<User>>,
    presenter_peer: Option<Id<User>>,
) -> (web_sys::Element, RwSignal<Option<PresenterIntent>>) {
    let open = RwSignal::new(if open {
        PresentMode::Presenting
    } else {
        PresentMode::Editing
    });
    let slide = RwSignal::new(SlidePosition::new(
        Pos::<Slide>::from_usize(index),
        Len::<Slide>::of(count),
    ));
    let notes = RwSignal::new(notes.to_owned());
    let broadcasting = RwSignal::new(if broadcasting {
        Broadcast::Broadcasting
    } else {
        Broadcast::Private
    });
    let following = RwSignal::new(following);
    let presenter_peer = RwSignal::new(presenter_peer);
    let fired = RwSignal::new(None::<PresenterIntent>);
    let on_intent = Callback::new(move |i: PresenterIntent| fired.set(Some(i)));
    let root = mount_in_container(move || {
        view! {
            <PresenterPanel
                open=open
                slide=slide
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
    // full-text replace (toile-text-editing char-CRDT).
    let (root, fired) = mount_presenter(true, 0, 1, "abc");
    type_into(&root, "Speaker notes", "abcd");
    assert_eq!(
        fired.get_untracked(),
        Some(PresenterIntent::EditNotes {
            base: "abc".to_owned(),
            delta: TextDelta {
                at: Pos::<Char>::new(3),
                delete: Len::<Char>::ZERO,
                insert: "d".to_owned(),
            },
        })
    );
    root.remove();
}

#[wasm_bindgen_test]
fn presenter_broadcast_button_fires_toggle() {
    let owner = Owner::new();
    owner.set();
    // Broadcasting off: clicking the button requests turning it on.
    let (root, fired) = mount_presenter_follow(true, 0, 3, "", false, None, None);
    assert!(click_by_aria(&root, "Broadcast position"));
    assert_eq!(
        fired.get_untracked(),
        Some(PresenterIntent::ToggleBroadcast(Broadcast::Broadcasting))
    );
    root.remove();
}

#[wasm_bindgen_test]
fn presenter_follow_button_hidden_without_presenter() {
    let owner = Owner::new();
    owner.set();
    // No peer is broadcasting -> the Follow affordance is absent.
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
    let peer = Id::<User>::mint();
    // A peer is broadcasting and we are not following: clicking follows it.
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

// The tests that wait for an effect, the notes area's seeding among them, live
// in `chrome_effects`.

// -- The deck panel (`toile-presentation`) ------------------------------------

/// A deck row for the fixtures below: an id and a title.
fn deck_frame(id: &str, name: &str) -> DeckFrame {
    DeckFrame {
        id: Id::<Frame>::from_text(id.to_owned()),
        name: name.to_owned(),
    }
}

/// Mount a `DeckPanel` over a deck and a pool of frames off it, returning the
/// container and the signal that captures emitted intents.
fn mount_deck(
    slides: Vec<DeckFrame>,
    off_deck: Vec<DeckFrame>,
) -> (web_sys::Element, RwSignal<Option<DeckIntent>>) {
    let state = RwSignal::new(PanelState::Open);
    let view = RwSignal::new(DeckView { slides, off_deck });
    let fired = RwSignal::new(None::<DeckIntent>);
    let on_intent = Callback::new(move |i: DeckIntent| fired.set(Some(i)));
    let root = mount_in_container(move || {
        view! { <DeckPanel state=state view=view on_intent=on_intent /> }
    });
    (root, fired)
}

/// Adding, reordering and removing each leave the panel as one intent
/// (`TOILE-FEAT-026`): the panel writes no order of its own.
#[wasm_bindgen_test]
fn deck_panel_edits_fire_intents() {
    let owner = Owner::new();
    owner.set();
    let (root, fired) = mount_deck(
        vec![deck_frame("a", "One"), deck_frame("b", "Two")],
        vec![deck_frame("c", "Three")],
    );

    assert!(click_by_aria(&root, "Move down"));
    assert_eq!(
        fired.get_untracked(),
        Some(DeckIntent::Shift(Pos::<Slide>::ZERO, DeckMove::Down)),
        "the first row's only reorder step is downwards"
    );

    assert!(click_by_aria(&root, "Remove from deck"));
    assert_eq!(
        fired.get_untracked(),
        Some(DeckIntent::Remove(Pos::<Slide>::ZERO))
    );

    let add: web_sys::HtmlElement = root
        .query_selector(".ambre-deck-add-btn")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    add.click();
    assert_eq!(
        fired.get_untracked(),
        Some(DeckIntent::Add(Id::<Frame>::from_text("c".to_owned())))
    );
    root.remove();
}

/// `TOILE-247`: the panel is where a frame is named, deck member or not. One
/// edit leaves as one intent naming that frame, so the engine writes one undo
/// item rather than one per keystroke (`TOILE-248`).
#[wasm_bindgen_test]
fn deck_panel_names_a_frame() {
    let owner = Owner::new();
    owner.set();
    let (root, fired) = mount_deck(vec![deck_frame("a", "One")], vec![deck_frame("c", "Three")]);

    let type_into = |sel: &str, text: &str| {
        let field: web_sys::HtmlInputElement = root
            .query_selector(sel)
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        field.set_value(text);
        field
            .dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
    };

    type_into(".ambre-deck-list .ambre-deck-name", "Agenda");
    assert_eq!(
        fired.get_untracked(),
        Some(DeckIntent::Rename(
            Id::<Frame>::from_text("a".to_owned()),
            "Agenda".to_owned()
        ))
    );

    // A frame off the deck is named the same way: it has no rank to address.
    type_into(".ambre-deck-pool .ambre-deck-name", "Roadmap");
    assert_eq!(
        fired.get_untracked(),
        Some(DeckIntent::Rename(
            Id::<Frame>::from_text("c".to_owned()),
            "Roadmap".to_owned()
        ))
    );
    root.remove();
}

/// A board with no deck says so rather than showing an empty list
/// (`TOILE-UI-342`).
#[wasm_bindgen_test]
fn deck_panel_empty_deck_invites_a_first_slide() {
    let owner = Owner::new();
    owner.set();
    let (root, _fired) = mount_deck(Vec::new(), vec![deck_frame("a", "One")]);
    assert!(root.query_selector(".ambre-deck-empty").unwrap().is_some());
    root.remove();
}

// -- The interop notifications (`toile-import-export`) ------------------------

/// Hand `event` to Toile's notifier, then mount a live toast host, returning
/// the container so the test can read what the user would see.
///
/// The notice goes up before the mount so the first `<For>` render carries it.
/// A later push reaches the DOM through an effect. The runtime queues that
/// effect on a microtask, so a test reading the container on the same turn
/// finds an empty host.
fn mount_notified(event: ToileEvent) -> web_sys::Element {
    let toasts = Toasts::new();
    ambre_toile::chrome::notify_event(
        &toasts,
        &Event::Viewport {
            document_id: Id::<Document>::mint(),
            event,
        },
    );
    mount_in_container(move || view! { <ToastHost toasts=toasts /> })
}

/// An import report reaches the user, and a lossy one says what it dropped
/// (`TOILE-IO-022`).
#[wasm_bindgen_test]
fn an_import_report_reaches_the_user() {
    let owner = Owner::new();
    owner.set();
    let mut report = ToileReport::new(Direction::Import, FileFormat::Svg);
    report.record_count(
        ToileConstruct::Element,
        Outcome::Carried,
        ambre_core::Count::<Occurrences>::new(4),
    );
    report.record(ToileConstruct::NonSolidPaint, Outcome::Dropped);
    let root = mount_notified(ToileEvent::Interop(InteropAnswer::ImportCompleted {
        report,
    }));
    let toast = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the report is on screen, not in the log");
    let text = toast.text_content().unwrap_or_default();
    assert!(text.contains("Imported 4 elements"), "{text}");
    assert!(text.contains("gradient"), "{text}");
    root.remove();
}

/// A file that never mapped says so: without a word the user cannot tell the
/// drop from a miss (`TOILE-IO-022`).
#[wasm_bindgen_test]
fn a_refused_file_reaches_the_user() {
    let owner = Owner::new();
    owner.set();
    let root = mount_notified(ToileEvent::Interop(InteropAnswer::failed(
        Direction::Import,
        InteropError::ParseFailed(ambre_core::FileRefusal::Malformed),
    )));
    let text = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the refusal is on screen")
        .text_content()
        .unwrap_or_default();
    assert!(text.contains("does not read"), "{text}");
    root.remove();
}

/// An export confirms which file left (`TOILE-IO-021`).
#[wasm_bindgen_test]
fn a_finished_export_names_its_file() {
    let owner = Owner::new();
    owner.set();
    let root = mount_notified(ToileEvent::Interop(InteropAnswer::ExportReady {
        file: ExportedFile {
            filename: ambre_client_core::ExportFilename::named(
                "board",
                ambre_client_core::ExportExtension::Svg,
            ),
            media_type: ambre_core::MediaType::from("image/svg+xml"),
            data: ambre_core::Payload::<ambre_client_core::FileBytes>::default(),
        },
        report: ToileReport::new(Direction::Export, FileFormat::Svg),
    }));
    let text = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the export is on screen")
        .text_content()
        .unwrap_or_default();
    assert!(text.contains("board.svg"), "{text}");
    root.remove();
}

/// An export that produced nothing says why (`TOILE-IO-048`). Without the toast
/// the gesture is indistinguishable from one that never arrived: no file lands,
/// and the board looks exactly as it did.
#[wasm_bindgen_test]
fn a_refused_export_says_why() {
    let owner = Owner::new();
    owner.set();
    let root = mount_notified(ToileEvent::Interop(InteropAnswer::failed(
        Direction::Export,
        InteropError::Stopped(ambre_core::StopCause::FontsLoading),
    )));
    let text = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the refusal is on screen")
        .text_content()
        .unwrap_or_default();
    assert!(text.contains("the fonts are still loading"), "{text}");
    root.remove();
}

/// Mount Toile's choosers over `docs`, with `here` open, and hand back the
/// dialogs' state and where a placed embed is published.
fn mount_choosers(
    docs: Vec<ambre_toile::chrome::DocChoice>,
    here: Id<Document>,
) -> (
    web_sys::Element,
    ambre_toile::chrome::ToileDialogs,
    RwSignal<Option<ambre_toile::chrome::PlacedEmbed>>,
) {
    use ambre_toile::chrome::{DeckSignals, ToileChoosers, ToileDialogs};
    let dialogs = ToileDialogs::new();
    let placed = RwSignal::new(None);
    let deck = DeckSignals {
        panel: RwSignal::new(PanelState::Closed),
        view: RwSignal::new(DeckView::default()),
        intent: RwSignal::new(None),
    };
    let set_link = RwSignal::new(None);
    let root = mount_in_container(move || {
        view! {
            <ToileChoosers
                dialogs=dialogs
                docs=Signal::derive(move || docs.clone())
                here=Signal::derive(move || Some(here))
                deck=deck
                placed=placed
                set_link=set_link
            />
        }
    });
    (root, dialogs, placed)
}

/// Click the button labelled `label` inside the element `selector` names.
fn click_in(root: &web_sys::Element, selector: &str, label: &str) -> bool {
    let buttons = root
        .query_selector(selector)
        .unwrap()
        .expect("the dialog")
        .query_selector_all("button")
        .unwrap();
    for i in 0..buttons.length() {
        let b: web_sys::HtmlElement = buttons.item(i).unwrap().dyn_into().unwrap();
        if b.text_content().as_deref() == Some(label) {
            b.click();
            return true;
        }
    }
    false
}

/// `TOILE-087`: the web dialog places the address it normalised, and keeps a
/// refused one in the dialog, beside the field, with nothing placed.
#[wasm_bindgen_test]
fn the_web_dialog_places_a_normalised_address_and_keeps_a_refusal() {
    use ambre_toile::chrome::{AddressError, EmbedTarget};
    let (root, dialogs, placed) = mount_choosers(Vec::new(), Id::<Document>::mint());
    dialogs.embed_web_open.set(ambre_chrome::Openness::Open);

    dialogs.embed_web_text.set("javascript:alert(1)".to_owned());
    assert!(click_in(&root, ".ambre-embed-web", "Embed"));
    assert_eq!(placed.get_untracked(), None, "a refused address was placed");
    assert_eq!(
        dialogs.embed_web_error.get_untracked(),
        Some(AddressError::NotHttps)
    );
    assert_eq!(
        dialogs.embed_web_open.get_untracked(),
        ambre_chrome::Openness::Open,
        "the refusal closed the dialog"
    );

    dialogs.embed_web_text.set("example.com".to_owned());
    assert!(click_in(&root, ".ambre-embed-web", "Embed"));
    let placed = placed.get_untracked().expect("the address was placed");
    assert_eq!(
        placed.target,
        EmbedTarget::Web(
            ambre_core::Validated::<ambre_toile::message::WebAddress>::parse("https://example.com")
                .expect("a safe address")
        )
    );
    assert_eq!(
        dialogs.embed_web_open.get_untracked(),
        ambre_chrome::Openness::Closed
    );
    root.remove();
}

/// `TOILE-086`: the document picker offers every document but the open one,
/// and a pick places a reference to the document it names.
#[wasm_bindgen_test]
fn the_document_picker_places_a_reference_and_never_offers_the_open_board() {
    use ambre_bus::DocType;
    use ambre_toile::chrome::{DocChoice, EmbedTarget};
    let here = Id::<Document>::mint();
    let other = Id::<Document>::mint();
    let docs = vec![
        DocChoice {
            document_id: here,
            doc_type: DocType::Toile,
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("This board").unwrap()),
        },
        DocChoice {
            document_id: other,
            doc_type: DocType::Trame,
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("Budget").unwrap()),
        },
    ];
    let (root, dialogs, placed) = mount_choosers(docs, here);
    dialogs.embed_doc_open.set(ambre_chrome::Openness::Open);

    let picker = root
        .query_selector(".ambre-embed-doc")
        .unwrap()
        .expect("the picker");
    let offered = picker
        .query_selector_all(".ambre-embed-docs button")
        .unwrap();
    assert_eq!(offered.length(), 1, "the open board was offered to itself");
    assert!(click_in(&root, ".ambre-embed-doc", "Budget"));
    let placed = placed.get_untracked().expect("the pick was placed");
    assert_eq!(
        placed.target,
        EmbedTarget::Document {
            document_id: other,
            doc_type: DocType::Trame
        }
    );
    assert_eq!(
        dialogs.embed_doc_open.get_untracked(),
        ambre_chrome::Openness::Closed
    );
    root.remove();
}

// -- The comments panel and the drop composer (chrome/comments.rs) -----------

fn summary(id: &str, preview: &str) -> CommentSummary {
    CommentSummary {
        id: Id::<Thread>::from_text(id.to_owned()),
        anchor: CommentAnchor::Point(kurbo::Point::new(10.0, 20.0)),
        resolved: ambre_core::ThreadState::Open,
        messages: Len::<ambre_toile::message::ThreadMessage>::new(1),
        preview: preview.to_owned(),
    }
}

/// The text of every element under `root` that matches `selector`.
fn texts(root: &web_sys::Element, selector: &str) -> Vec<String> {
    let found = root.query_selector_all(selector).unwrap();
    (0..found.length())
        .filter_map(|i| found.item(i)?.text_content())
        .collect()
}

/// `TOILE-UI-023`: the panel shows what the board answered and nothing else.
/// A new answer replaces the rows whole, and a click on a row leaves as an
/// intent. The panel changes nothing of its own.
#[wasm_bindgen_test]
async fn the_comment_panel_shows_what_the_board_answered() {
    let owner = Owner::new();
    owner.set();
    let state = RwSignal::new(PanelState::Open);
    let list = RwSignal::new(vec![summary("t1", "first"), summary("t2", "second")]);
    let open = RwSignal::new(None::<CommentThread>);
    let fired = RwSignal::new(None::<CommentIntent>);
    let on_intent = Callback::new(move |i| fired.set(Some(i)));
    let root = mount_in_container(move || {
        view! { <CommentsPanel state=state list=list open=open on_intent=on_intent /> }
    });
    assert_eq!(texts(&root, ".ambre-comments-preview"), ["first", "second"]);

    list.set(vec![summary("t3", "third")]);
    open.set(Some(CommentThread {
        id: Id::<Thread>::from_text("t3".to_owned()),
        anchor: CommentAnchor::Point(kurbo::Point::new(10.0, 20.0)),
        resolved: ambre_core::ThreadState::Open,
        messages: vec![CommentMessage {
            author: Id::<User>::mint(),
            ts: Instant::<Authored>::new(0),
            body: "read back".to_owned(),
        }],
    }));
    leptos::task::tick().await;
    assert_eq!(texts(&root, ".ambre-comments-preview"), ["third"]);
    assert_eq!(texts(&root, ".ambre-comments-body"), ["read back"]);

    assert!(click_in(&root, ".ambre-comments-list", "third1"));
    assert_eq!(
        fired.get_untracked(),
        Some(CommentIntent::Focus {
            thread: Id::<Thread>::from_text("t3".to_owned()),
            anchor: CommentAnchor::Point(kurbo::Point::new(10.0, 20.0)),
        })
    );
    leptos::task::tick().await;
    assert_eq!(
        texts(&root, ".ambre-comments-preview"),
        ["third"],
        "the click changed no row"
    );

    open.set(None);
    leptos::task::tick().await;
    assert!(texts(&root, ".ambre-comments-body").is_empty());
    root.remove();
}

/// `TOILE-UI-307`: a dropped comment opens its composer at the click, with
/// the field focused, so the user types where they clicked.
#[wasm_bindgen_test]
async fn a_dropped_comment_opens_its_composer_where_it_clicked() {
    let owner = Owner::new();
    owner.set();
    let draft = RwSignal::new(None::<CommentDraft>);
    let on_intent = Callback::new(move |_: CommentIntent| {});
    let root = mount_in_container(move || {
        view! { <CommentComposer draft=draft on_intent=on_intent /> }
    });
    assert!(
        root.query_selector(".ambre-comment-composer")
            .unwrap()
            .is_none()
    );

    draft.set(Some(CommentDraft {
        anchor: CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        at: ambre_draw::Point::new(120.0, 80.0),
    }));
    leptos::task::tick().await;
    let composer = root
        .query_selector(".ambre-comment-composer")
        .unwrap()
        .expect("the composer opened");
    let style = composer.get_attribute("style").unwrap_or_default();
    assert!(
        style.contains("left: 120px") && style.contains("top: 80px"),
        "at the click: {style}"
    );
    let field = root
        .query_selector(".ambre-comment-composer-input")
        .unwrap()
        .expect("the field");
    assert_eq!(
        document().active_element(),
        Some(field),
        "the field has the focus"
    );
    root.remove();
}

/// `TOILE-UI-083`: the editing chrome leaves the DOM while the board presents,
/// and comes back when the presentation ends.
#[wasm_bindgen_test]
async fn the_editing_chrome_leaves_while_the_board_presents() {
    let present = RwSignal::new(PresentMode::Editing);
    let active = RwSignal::new(Tool::Select);
    let root = mount_in_container(move || {
        view! {
            <EditingSurfaces present=present>
                <ToolPalette active=active />
            </EditingSurfaces>
        }
    });
    let shown = |root: &web_sys::Element| root.child_element_count() > 0;
    assert!(shown(&root), "the palette is mounted");
    present.set(PresentMode::Presenting);
    leptos::task::tick().await;
    assert!(!shown(&root), "a presentation unmounts the palette");
    present.set(PresentMode::Editing);
    leptos::task::tick().await;
    assert!(shown(&root), "the palette comes back");
    root.remove();
}

/// A relay over fresh signals, as the launcher builds one.
fn relay() -> ToileRelay {
    ToileRelay {
        sel_stats: RwSignal::new(None),
        presenter_slide: RwSignal::new(SlidePosition::default()),
        presenter_notes: RwSignal::new(String::new()),
        presenter_frame: RwSignal::new(None),
        broadcasting: RwSignal::new(Broadcast::Private),
        following: RwSignal::new(None),
        presenter_peer: RwSignal::new(None),
    }
}

/// `ev` as the launcher hands it to the relay.
fn board_event(ev: ToileEvent) -> Event {
    Event::Viewport {
        document_id: Id::<Document>::mint(),
        event: ev,
    }
}

/// One peer of the live set, broadcasting `slide` or not.
fn peer(user_id: Id<User>, slide: Option<u32>) -> PresenceUpdate {
    PresenceUpdate {
        user_id,
        presence: PresencePayload {
            present_slide: slide.map(Pos::<Slide>::new),
            ..Default::default()
        },
    }
}

/// `TOILE-UI-315`: the presenter signal names a peer only while that peer
/// broadcasts a slide. A peer that stops, or leaves the live set, names nobody.
#[wasm_bindgen_test]
fn the_presenter_is_named_only_while_it_broadcasts() {
    let relay = relay();
    let (alice, bob) = (Id::<User>::mint(), Id::<User>::mint());
    relay.absorb(&board_event(ToileEvent::PeerPresence(vec![
        peer(bob, None),
        peer(alice, Some(2)),
    ])));
    assert_eq!(relay.presenter_peer.get_untracked(), Some(alice));
    relay.absorb(&board_event(ToileEvent::PeerPresence(vec![
        peer(bob, None),
        peer(alice, None),
    ])));
    assert_eq!(relay.presenter_peer.get_untracked(), None, "alice stopped");
    relay.absorb(&board_event(ToileEvent::PeerPresence(vec![peer(
        alice,
        Some(0),
    )])));
    relay.absorb(&board_event(ToileEvent::PeerPresence(Vec::new())));
    assert_eq!(relay.presenter_peer.get_untracked(), None, "alice left");
}

/// `TOILE-FEAT-091`: a note edit with no frame on screen becomes no command.
/// Once a slide names its frame, the same edit lands on that frame.
#[wasm_bindgen_test]
fn a_note_edit_with_no_frame_on_screen_is_refused() {
    let relay = relay();
    let edit = || {
        ToileIntent::Present(PresenterIntent::EditNotes {
            base: String::new(),
            delta: TextDelta {
                at: Pos::<Char>::ZERO,
                delete: Len::<Char>::ZERO,
                insert: "hi".to_owned(),
            },
        })
    };
    assert!(
        relay.command(edit()).is_none(),
        "an edit with no frame left"
    );
    let frame = Id::<Frame>::from_text("f".to_owned());
    relay.presenter_frame.set(Some(frame.clone()));
    assert!(matches!(
        relay.command(edit()),
        Some(ToileCommand::EditNotes { frame: f, .. }) if f == frame
    ));
}

/// Load the shell stylesheet into the page, as the app does, so the floating
/// surfaces take their real place and size. Remove it with the returned node.
fn with_shell_stylesheet() -> web_sys::Element {
    let sheet = document().create_element("style").unwrap();
    sheet.set_text_content(Some(include_str!("../../ambre-app/styles.css")));
    document().head().unwrap().append_child(&sheet).unwrap();
    sheet
}

/// Let the page lay out and deliver its resize observations: one reactive
/// tick, then two animation frames.
async fn settle() {
    leptos::task::tick().await;
    for _ in 0..2 {
        let frame = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .request_animation_frame(&resolve)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(frame).await.unwrap();
    }
}

/// `ZOOM-011`: the palette publishes the right edge it covers, as the page
/// lays it out. The edge tracks a change of size, and clears when the palette
/// hides.
#[wasm_bindgen_test]
async fn the_palette_publishes_the_edge_it_covers_and_clears_it_when_it_hides() {
    let sheet = with_shell_stylesheet();
    let active = RwSignal::new(Tool::Select);
    let cover = RwSignal::new(ambre_toile::ui::cover::Cover::default());
    let root = mount_in_container(move || view! { <ToolPalette active=active cover=cover /> });
    settle().await;
    let palette: web_sys::HtmlElement = root
        .query_selector(".ambre-tool-palette")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let measured = |cover: RwSignal<ambre_toile::ui::cover::Cover>| {
        cover
            .get_untracked()
            .palette_right
            .map(|right| f64::from(right.get()))
    };
    let right = palette.get_bounding_client_rect().right();
    assert!(
        right > 8.0,
        "the palette sits right of the page edge: {right}"
    );
    assert!((measured(cover).unwrap() - right).abs() < 0.5);

    palette.style().set_property("width", "120px").unwrap();
    settle().await;
    let wider = palette.get_bounding_client_rect().right();
    assert!(wider > right);
    assert!((measured(cover).unwrap() - wider).abs() < 0.5);

    palette.style().set_property("display", "none").unwrap();
    settle().await;
    assert_eq!(measured(cover), None);

    root.remove();
    sheet.remove();
}

/// `ZOOM-011`: the selection bar publishes how far it reaches above the
/// selection, its height and its lift, and clears it once it leaves.
#[wasm_bindgen_test]
async fn the_selection_bar_publishes_its_reach_and_clears_it_when_it_leaves() {
    let sheet = with_shell_stylesheet();
    let bar = RwSignal::new(Some(FloatBar {
        at: css_point(400.0, 300.0),
        count: Count::<Elements>::new(1),
        groups: GroupHeld::NoGroup,
        locks: LockMix::Unlocked,
        fill: None,
        mixed: Default::default(),
    }));
    let model = RwSignal::new(None::<StyleModel>);
    let cover = RwSignal::new(ambre_toile::ui::cover::Cover::default());
    let on_command = Callback::new(move |_id: StaticKey<Commands>| {});
    let on_style = Callback::new(move |_e: StyleEdit| {});
    let root = mount_in_container(move || {
        view! {
            <SelectionToolbar bar=bar model=model on_command=on_command on_style=on_style cover=cover />
        }
    });
    settle().await;
    let selbar = root.query_selector(".ambre-selbar").unwrap().unwrap();
    let rect = selbar.get_bounding_client_rect();
    let reach = f64::from(cover.get_untracked().bar_reach.unwrap().get());
    assert!((reach - (300.0 - rect.top())).abs() < 0.5);
    // The reach is the bar's own height and the lift above the selection.
    assert!(
        reach > rect.height(),
        "reach {reach}, height {}",
        rect.height()
    );

    bar.set(None);
    settle().await;
    assert_eq!(cover.get_untracked().bar_reach, None);

    root.remove();
    sheet.remove();
}

// -- The context panel (`ambre-spec-shell.md` section 2) ---------------------

/// A style model of `kind`, nothing mixed.
fn styled(kind: ambre_toile::ui::style::Styled) -> StyleModel {
    StyleModel {
        base: ambre_toile::message::ElementStyle::default(),
        mixed: Default::default(),
        kind,
    }
}

/// Mount the board's context panel on `model`. The scope holds a locked and
/// an unlocked member of a group of two, so every conditional control shows.
/// Return the container.
fn mount_toile_panel(model: Option<StyleModel>) -> web_sys::Element {
    use ambre_chrome::{ContextPanel, DockSide, Fold, PanelPrefs, PanelWidth};
    let model = RwSignal::new(model);
    let scope = RwSignal::new(CtxScope {
        site: ClickSite::Selection,
        count: Count::<Elements>::new(2),
        groups: GroupHeld::Group,
        locks: LockMix::Both,
        ..Default::default()
    });
    let content = ambre_toile::chrome::toile_panel(ambre_toile::chrome::ToilePanelSignals {
        model,
        scope,
        on_command: Callback::new(|_| {}),
        restyle: RwSignal::new(None),
    });
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    mount_in_container(move || {
        view! { <ContextPanel prefs=prefs title=content.title sections=content.sections /> }
    })
}

/// With nothing selected the panel shows the board. With a selection it shows
/// the sections that apply to it, in the one frame.
#[wasm_bindgen_test]
async fn the_board_panel_shows_the_board_then_what_applies_to_the_selection() {
    use ambre_toile::ui::panel::{self, PanelKind};
    use ambre_toile::ui::style::Styled;
    let board = mount_toile_panel(None);
    leptos::task::tick().await;
    assert_eq!(
        board
            .query_selector(".ambre-ctx-title")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap(),
        "Board"
    );
    let markers = ambre_chrome::reach_markers(&board);
    assert!(
        ambre_chrome::commands::Reach::Command(menu::SELECT_ALL).matches_marker(&markers[0]),
        "{markers:?}"
    );
    board.remove();

    for (kind, model) in [
        (PanelKind::Shape, styled(Styled::Elements)),
        (PanelKind::Connector, styled(Styled::Arrow)),
    ] {
        let root = mount_toile_panel(Some(model));
        leptos::task::tick().await;
        let shown: Vec<String> = (0..root
            .query_selector_all(".ambre-ctx-section")
            .unwrap()
            .length())
            .map(|i| {
                root.query_selector_all(".ambre-ctx-section")
                    .unwrap()
                    .item(i)
                    .unwrap()
                    .dyn_into::<web_sys::Element>()
                    .unwrap()
                    .get_attribute("data-section")
                    .unwrap()
            })
            .collect();
        let expected: Vec<&str> = kind.sections().iter().map(|k| k.as_str()).collect();
        assert_eq!(shown, expected, "{kind:?}");
        // Every control of the panel names a declared command or property.
        for marker in ambre_chrome::reach_markers(&root) {
            let declared = ambre_toile::ui::commands::COMMANDS
                .iter()
                .any(|c| ambre_chrome::commands::Reach::Command(c.id).matches_marker(&marker))
                || panel::PROPERTIES
                    .iter()
                    .any(|p| ambre_chrome::commands::Reach::Property(p.id).matches_marker(&marker));
            assert!(declared, "{kind:?}: {marker} is declared");
        }
        root.remove();
    }
}

/// Each entry of the board's context bar is a control the panel shows for the
/// same kind. The bar shows its declared list alone.
#[wasm_bindgen_test]
async fn each_entry_of_the_board_bar_is_a_control_of_the_panel() {
    use ambre_toile::ui::panel::PanelKind;
    use ambre_toile::ui::style::Styled;
    let root = mount_toile_panel(Some(styled(Styled::Elements)));
    leptos::task::tick().await;
    let panel = ambre_chrome::reach_markers(&root);
    for entry in PanelKind::Shape.bar() {
        assert!(
            panel.iter().any(|m| entry.matches_marker(m)),
            "{entry:?} is in the panel: {panel:?}"
        );
    }
    root.remove();

    let bar = RwSignal::new(Some(FloatBar {
        at: css_point(40.0, 20.0),
        count: Count::<Elements>::new(2),
        groups: GroupHeld::Group,
        locks: LockMix::Both,
        fill: None,
        mixed: Default::default(),
    }));
    let model = RwSignal::new(Some(styled(Styled::Elements)));
    let toolbar = mount_in_container(move || {
        view! {
            <SelectionToolbar bar=bar model=model on_command=Callback::new(|_| {}) on_style=Callback::new(|_| {}) />
        }
    });
    let shown = ambre_chrome::reach_markers(&toolbar);
    assert_eq!(shown.len(), PanelKind::Shape.bar().len());
    for (marker, entry) in shown.iter().zip(PanelKind::Shape.bar()) {
        assert!(entry.matches_marker(marker), "{marker} is {entry:?}");
    }
    toolbar.remove();
}
