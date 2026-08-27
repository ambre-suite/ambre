//! Leptos UI tests for the Trame chrome (Epic `ET`,
//! `ambre-spec-testing.md`, the "wasm" tier).
//!
//! The chrome (`ambre_trame::chrome`) is `cfg(target_arch = "wasm32")` -- leptos
//! CSR + DOM (`web-sys`/`wasm-bindgen`) that does not compile on the host. So its
//! tests run under the wasm target via the `wasm-bindgen-test-runner`, wrapped by
//! `wasm-pack test --headless --chrome crates/ambre-trame` (`just test-wasm`).
//!
//! Two belts, per the story's DoD:
//!   - the **reactive / presentation belt** -- the pure inbound-event mirrors
//!     (`apply_style_event`, `mirror_event`) and the `CommandId -> AppCommand`
//!     mapping, exercised over real leptos signals but no DOM;
//!   - at least one **DOM `wasm-bindgen-test`** -- a component mounted into the
//!     real document, asserted through `web-sys`.
//!
//! On the host target the whole file is empty (the `cfg` gate below), so the
//! native tier (`just test-native`) skips it cleanly.
#![cfg(target_arch = "wasm32")]

use ambre_bus::{
    AppCommand, AppEvent, CellFormat, CellStyle, DocumentId, HAlign, TrameCommand, TrameEvent,
    UiEvent, VAlign,
};
use ambre_canvas::PerfSnapshot;
use ambre_chrome::CommandStates;
use ambre_trame::chrome::{
    self, BOLD, FindMode, FindQuery, FindReplace, FindReplaceState, REDO, ReorderSheet,
    ReplaceRequest, SelStats, SheetTabs, SortApply, SortDialog, SortDialogState, StatusBar,
    StyleState, UNDO,
};
use leptos::mount::mount_to;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

// Every test touches the DOM (or at least the leptos reactive graph backed by
// the browser), so run them inside headless Chrome rather than Node.
wasm_bindgen_test_configure!(run_in_browser);

/// Wrap a `StyleApplied` event the way the launcher relays it, so the mirror
/// under test sees exactly the shape it does in production.
fn style_applied(
    style: CellStyle,
    bold: bool,
    italic: bool,
    merged: bool,
    numeric: bool,
) -> UiEvent {
    UiEvent::Viewport {
        document_id: DocumentId::new(),
        event: AppEvent::Trame(TrameEvent::StyleApplied {
            style,
            bold,
            italic,
            merged,
            numeric,
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Reactive / presentation belt
// ─────────────────────────────────────────────────────────────────────────────

/// `apply_style_event` fans a `StyleApplied` out into every `StyleState` signal
/// the formatting toolbar subscribes to. This is the single style channel, so
/// bold/italic/merged (reported beside the style) land here too.
#[wasm_bindgen_test]
fn style_applied_mirrors_into_style_state() {
    // A reactive owner is required to allocate signals outside a component.
    let owner = Owner::new();
    owner.set();

    let state = StyleState::new();
    let style = CellStyle {
        underline: true,
        wrap: true,
        h_align: HAlign::Right,
        v_align: VAlign::Bottom,
        format: CellFormat::Percent,
        ..CellStyle::default()
    };
    chrome::apply_style_event(&state, &style_applied(style, true, false, true, true));

    assert!(
        state.bold.get(),
        "bold reported separately must be mirrored"
    );
    assert!(!state.italic.get());
    assert!(
        state.merged.get(),
        "merge pressed-state drives the merge toggle"
    );
    assert!(state.underline.get());
    assert!(state.wrap.get());
    assert!(
        state.numeric.get(),
        "numeric gates Auto-alignment resolution"
    );
    assert_eq!(state.h_align.get(), HAlign::Right);
    assert_eq!(state.v_align.get(), VAlign::Bottom);
    assert_eq!(state.format.get(), CellFormat::Percent);
}

/// Any non-`StyleApplied` event leaves the `StyleState` untouched: the launcher
/// calls the mirror for every relayed event, so it must be a strict no-op for
/// the rest.
#[wasm_bindgen_test]
fn style_state_ignores_unrelated_events() {
    let owner = Owner::new();
    owner.set();

    let state = StyleState::new();
    state.bold.set(true);
    let ev = UiEvent::Viewport {
        document_id: DocumentId::new(),
        event: AppEvent::Trame(TrameEvent::DataReset),
    };
    chrome::apply_style_event(&state, &ev);
    assert!(
        state.bold.get(),
        "DataReset must not clobber the style state"
    );
}

/// `mirror_event` projects the authoritative Undo/Redo availability from
/// `TrameEvent::UndoState` into the command store the toolbar buttons read for
/// their `disabled` state.
#[wasm_bindgen_test]
fn undo_state_mirrors_into_command_store() {
    let owner = Owner::new();
    owner.set();

    let store = CommandStates::new(chrome::COMMANDS.iter().map(|c| c.id));
    chrome::init_command_states(&store);
    // A fresh document starts with an empty history.
    assert!(!store.enabled_signal(UNDO).unwrap().get());
    assert!(!store.enabled_signal(REDO).unwrap().get());

    let ev = UiEvent::Viewport {
        document_id: DocumentId::new(),
        event: AppEvent::Trame(TrameEvent::UndoState {
            can_undo: true,
            can_redo: false,
        }),
    };
    chrome::mirror_event(&store, &ev);
    assert!(store.enabled_signal(UNDO).unwrap().get());
    assert!(!store.enabled_signal(REDO).unwrap().get());
}

/// The `CommandId -> AppCommand` mapping: structural intents become a concrete
/// `TrameCommand`, while UI-only intents (style toggles the launcher intercepts)
/// map to `None`.
#[wasm_bindgen_test]
fn command_to_app_maps_intents() {
    assert!(matches!(
        chrome::command_to_app(UNDO),
        Some(AppCommand::Trame(TrameCommand::Undo))
    ));
    assert!(matches!(
        chrome::command_to_app(REDO),
        Some(AppCommand::Trame(TrameCommand::Redo))
    ));
    // BOLD is intercepted into a SetCellStyle patch by the launcher; no direct
    // AppCommand here.
    assert!(chrome::command_to_app(BOLD).is_none());
    // "Sort range..." is a UI-only intent the launcher intercepts to open the
    // SortDialog (the dialog emits SortSelectionMulti itself); no direct mapping.
    assert!(chrome::command_to_app(chrome::SORT_RANGE_MULTI).is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// DOM belt
// ─────────────────────────────────────────────────────────────────────────────

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `view` into a fresh `<div>` appended to the body and return it.
///
/// DOM tests must clean up after themselves, but they must NOT wipe the whole
/// body: the `wasm-bindgen-test` browser harness keeps its own
/// `<pre id="output">` / `<pre id="console_output">` elements there, and routes
/// every `console.log` (including the final "test result: ..." line the runner
/// polls for) through them. Clearing the body (`set_inner_html("")`) deletes
/// those elements, so the next log throws `appendChild` on null, the summary is
/// never written, and the runner times out (SIGKILL). Scoping each test to its
/// own container -- removed via `container.remove()` -- leaves the harness
/// elements untouched. `mount_to(..).forget()` keeps the view mounted for the
/// assertions (like `mount_to_body`); the container removal tears it down.
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

/// DOM test: mounting the `StatusBar` renders the app label and the selection
/// aggregates, and only shows the numeric aggregates when a number is selected.
#[wasm_bindgen_test]
fn status_bar_renders_label_and_aggregates() {
    let container = mount_in_container(|| {
        let stats = RwSignal::new(Some(SelStats {
            sum: 6.0,
            avg: 2.0,
            count: 3,
            count_num: 3,
            min: 1.0,
            max: 3.0,
        }));
        let perf = RwSignal::new(PerfSnapshot::default());
        let zoom = RwSignal::new(1.0_f64);
        view! { <StatusBar stats=stats perf=perf zoom=zoom /> }
    });

    let text = container.text_content().unwrap_or_default();
    assert!(text.contains("Trame"), "status label missing: {text}");
    assert!(text.contains("Sum: 6"), "sum aggregate missing: {text}");
    assert!(text.contains("Count: 3"), "count aggregate missing: {text}");

    container.remove();
}

/// DOM test: the `FindReplace` panel mounts hidden when closed and, once opened
/// in find mode, exposes the "Find" input without the replace-only field.
#[wasm_bindgen_test]
fn find_replace_panel_opens_in_find_mode() {
    let doc = document();
    let container = mount_in_container(|| {
        let state = RwSignal::new(None::<FindReplaceState>);
        let result = RwSignal::new(None::<(usize, usize)>);
        let noop_find = Callback::new(|_: FindQuery| {});
        let noop_replace = Callback::new(|_: ReplaceRequest| {});
        // Open the panel in find mode after the initial render so the closed ->
        // open transition is what the DOM reflects.
        state.set(Some(FindReplaceState {
            mode: FindMode::Find,
        }));
        view! {
            <FindReplace
                state=state
                result=result
                on_find=noop_find
                on_replace=noop_replace
                on_replace_all=noop_replace
            />
        }
    });

    let panel = doc.query_selector(".ambre-find-panel").unwrap();
    assert!(panel.is_some(), "find panel should be mounted when open");
    let find_input = doc.query_selector(".ambre-find-input").unwrap();
    assert!(find_input.is_some(), "find input should be present");
    // Find mode hides the "Replace" button (replace-only row).
    let replace_btn = doc.query_selector(".ambre-find-replace").unwrap();
    assert!(
        replace_btn.is_none(),
        "replace row must be hidden in find mode"
    );

    container.remove();
}

// Multi-column sort dialog (`trame-sort`): opens over a selection snapshot,
// offers one level per "Add sort level", and emits the chosen levels on Sort.
#[wasm_bindgen_test]
fn sort_dialog_adds_levels_and_emits_on_apply() {
    let doc = document();
    // Capture what the dialog emits when the user clicks "Sort". A signal (not an
    // Rc<RefCell>) so the callback stays Send + Sync as `Callback::new` requires.
    let emitted = RwSignal::new(None::<SortApply>);

    let container = mount_in_container(move || {
        let state = RwSignal::new(None::<SortDialogState>);
        let on_apply = Callback::new(move |req: SortApply| {
            emitted.set(Some(req));
        });
        // Open over a three-column selection (A..C), so the picker offers 3 cols.
        state.set(Some(SortDialogState {
            r1: 0,
            c1: 0,
            r2: 4,
            c2: 2,
        }));
        view! { <SortDialog state=state on_apply=on_apply /> }
    });

    // The dialog is mounted, with at least one sort level rendered.
    assert!(
        doc.query_selector(".ambre-sort-dialog").unwrap().is_some(),
        "sort dialog should be mounted when open"
    );
    assert!(
        doc.query_selector(".ambre-sort-level").unwrap().is_some(),
        "starts with a (primary) sort level"
    );

    // "Add sort level" appends a second level (asserted through the emitted
    // request below, which reads the levels signal directly).
    let add = doc
        .query_selector(".ambre-sort-add")
        .unwrap()
        .expect("add button present")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    add.click();

    // Toggle the header-row checkbox so the emitted request carries it.
    let header = doc
        .query_selector(".ambre-sort-header-toggle input")
        .unwrap()
        .expect("header toggle present")
        .dyn_into::<web_sys::HtmlInputElement>()
        .unwrap();
    header.click();

    // Click "Sort": the dialog emits the levels (distinct columns) + has_header.
    let apply = doc
        .query_selector(".ambre-sort-apply")
        .unwrap()
        .expect("sort button present")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    apply.click();

    let req = emitted.get_untracked().expect("Sort emitted a request");
    assert!(req.has_header, "header toggle carried into the request");
    // Two distinct levels (the add-level default picks the next unused column).
    assert_eq!(req.sort_keys.len(), 2, "both levels emitted");
    assert_ne!(
        req.sort_keys[0].col, req.sort_keys[1].col,
        "distinct columns: a repeated column is dropped"
    );

    container.remove();
}

// ─────────────────────────────────────────────────────────────────────────────
// Context-menu scope (`trame-context-menu`): the grouped/iconed entries and the
// header-scoped layout that drops the cell-only clipboard + merge groups.
// ─────────────────────────────────────────────────────────────────────────────

/// Collect the command labels of an entry list, skipping separators.
fn item_commands(entries: &[ambre_chrome::MenuEntry]) -> Vec<ambre_chrome::commands::CommandId> {
    entries
        .iter()
        .filter_map(|e| match e {
            ambre_chrome::MenuEntry::Item { command, .. } => Some(*command),
            ambre_chrome::MenuEntry::Separator => None,
        })
        .collect()
}

#[wasm_bindgen_test]
fn cell_menu_has_clipboard_and_merge() {
    let entries = chrome::context_entries_for(chrome::CtxScope::Cell, false);
    let cmds = item_commands(&entries);
    // Clipboard + merge are cell-only; structural + sort/filter are shared.
    for expected in [
        chrome::CUT,
        chrome::COPY,
        chrome::PASTE,
        chrome::MERGE,
        chrome::UNMERGE,
    ] {
        assert!(cmds.contains(&expected), "cell menu must list {expected:?}");
    }
    // Separators group the entries (clipboard | structural | sort/filter | merge).
    let seps = entries
        .iter()
        .filter(|e| matches!(e, ambre_chrome::MenuEntry::Separator))
        .count();
    assert_eq!(seps, 3, "cell menu has three group separators");
}

#[wasm_bindgen_test]
fn column_menu_keeps_column_ops_drops_cell_and_row_ops() {
    let entries = chrome::context_entries_for(chrome::CtxScope::Column, false);
    let cmds = item_commands(&entries);
    // Column-axis structural + sort/filter survive...
    for expected in [
        chrome::INSERT_COL_LEFT,
        chrome::INSERT_COL_RIGHT,
        chrome::DELETE_COLS,
        chrome::SORT_RANGE_ASC,
        chrome::FILTER_COLUMN,
    ] {
        assert!(
            cmds.contains(&expected),
            "column menu must list {expected:?}"
        );
    }
    // ...the row-only inserts, the cell clipboard and merge do not.
    for absent in [
        chrome::INSERT_ROW_ABOVE,
        chrome::DELETE_ROWS,
        chrome::CUT,
        chrome::MERGE,
    ] {
        assert!(!cmds.contains(&absent), "column menu must drop {absent:?}");
    }
}

#[wasm_bindgen_test]
fn row_menu_keeps_row_ops_drops_column_inserts() {
    let entries = chrome::context_entries_for(chrome::CtxScope::Row, false);
    let cmds = item_commands(&entries);
    for expected in [
        chrome::INSERT_ROW_ABOVE,
        chrome::INSERT_ROW_BELOW,
        chrome::DELETE_ROWS,
        chrome::SORT_RANGE_ASC,
    ] {
        assert!(cmds.contains(&expected), "row menu must list {expected:?}");
    }
    for absent in [chrome::INSERT_COL_LEFT, chrome::DELETE_COLS, chrome::CUT] {
        assert!(!cmds.contains(&absent), "row menu must drop {absent:?}");
    }
}

#[wasm_bindgen_test]
fn corner_menu_has_sheet_sort_only() {
    let entries = chrome::context_entries_for(chrome::CtxScope::Corner, false);
    let cmds = item_commands(&entries);
    assert!(cmds.contains(&chrome::SORT_SHEET_ASC));
    // No per-row/column structural or clipboard ops on a whole-sheet selection.
    for absent in [
        chrome::INSERT_COL_LEFT,
        chrome::INSERT_ROW_ABOVE,
        chrome::CUT,
        chrome::MERGE,
    ] {
        assert!(!cmds.contains(&absent), "corner menu must drop {absent:?}");
    }
}

#[wasm_bindgen_test]
fn filter_entry_toggles_with_filter_state() {
    // No active filter -> "Filter by column" offered, "Clear filter" absent.
    let none = item_commands(&chrome::context_entries_for(chrome::CtxScope::Cell, false));
    assert!(none.contains(&chrome::FILTER_COLUMN));
    assert!(!none.contains(&chrome::CLEAR_FILTER));
    // Active filter -> "Remove filter" offered, "Filter by column" absent.
    let active = item_commands(&chrome::context_entries_for(chrome::CtxScope::Cell, true));
    assert!(active.contains(&chrome::CLEAR_FILTER));
    assert!(!active.contains(&chrome::FILTER_COLUMN));
}

/// DOM test: the `SheetTabs` strip renders one `.tab` per sheet with the active
/// one flagged, a click on another tab emits `on_activate`, and the trailing
/// "+" emits `on_create`. Guards the canvas->DOM migration of the tab strip.
#[wasm_bindgen_test]
fn sheet_tabs_render_active_and_emit_intents() {
    let activated = RwSignal::new(None::<String>);
    let created = RwSignal::new(0_u32);

    let container = mount_in_container(move || {
        let sheets = RwSignal::new(vec![
            ("s0".to_owned(), "One".to_owned()),
            ("s1".to_owned(), "Two".to_owned()),
        ]);
        let active = RwSignal::new("s1".to_owned());
        view! {
            <SheetTabs
                sheets=sheets
                active=active
                on_activate=Callback::new(move |id| activated.set(Some(id)))
                on_create=Callback::new(move |()| created.update(|n| *n += 1))
                on_rename=Callback::new(move |_: (String, String)| {})
                on_delete=Callback::new(move |_: String| {})
                on_reorder=Callback::new(move |_: ReorderSheet| {})
            />
        }
    });

    let doc = document();
    // Two tabs, and exactly one carries the `.active` class (the second sheet).
    let tabs = container.query_selector_all(".tab").unwrap();
    assert_eq!(tabs.length(), 2, "expected one button per sheet");
    let active_tabs = container.query_selector_all(".tab.active").unwrap();
    assert_eq!(active_tabs.length(), 1, "exactly one active tab");
    let active_el: web_sys::Element = active_tabs.item(0).unwrap().dyn_into().unwrap();
    assert_eq!(active_el.text_content().unwrap_or_default(), "Two");

    // Clicking the first (inactive) tab emits `on_activate("s0")`.
    let first: web_sys::HtmlElement = tabs.item(0).unwrap().dyn_into().unwrap();
    first.click();
    assert_eq!(activated.get_untracked().as_deref(), Some("s0"));

    // Clicking the trailing "+" emits `on_create`.
    let add: web_sys::HtmlElement = doc
        .query_selector(".tab-add")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    add.click();
    assert_eq!(created.get_untracked(), 1);

    container.remove();
}
