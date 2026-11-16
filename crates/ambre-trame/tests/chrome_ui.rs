//! Leptos UI tests for the Trame chrome (Epic `ET`,
//! `ambre-spec-testing.md`, the "wasm" tier).
//!
//! The chrome (`ambre_trame::chrome`) is `cfg(target_arch = "wasm32")` -- leptos
//! CSR + DOM (`web-sys`/`wasm-bindgen`) that does not compile on the host. So its
//! tests run under the wasm target via the `wasm-bindgen-test-runner`, in
//! headless Chrome (`just test-wasm`).
//!
//! Two belts, per the story's DoD:
//!   - the **reactive / presentation belt** -- the pure inbound-event mirrors
//!     (`apply_style_event`, `mirror_event`) and the `StaticKey<Commands> -> AppCommand`
//!     mapping, exercised over real leptos signals but no DOM;
//!   - at least one **DOM `wasm-bindgen-test`** -- a component mounted into the
//!     real document, asserted through `web-sys`.
//!
//! On the host target the whole file is empty (the `cfg` gate below), so the
//! native tier (`just test-native`) skips it cleanly.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::app::find::{FindQuery, Match};
use ambre_bus::{Col, Row, UiEvent};
use ambre_canvas::PerfSnapshot;
use ambre_chrome::CommandStates;
use ambre_commands::Locale;
use ambre_core::id::Sheet;
use ambre_core::id::{Document, Id};
use ambre_core::index::{Len, Pos};
use ambre_trame::chrome::{
    self, BOLD, COPY, CUT, EXPORT_CSV, FIND, FindMode, FindReplace, FindReplaceState, FormatBar,
    IMPORT_CSV, ITALIC, MERGE, PASTE, REDO, ReorderSheet, ReplaceRequest, SAVE_FILE, SelStats,
    SheetTabs, SortApply, SortDialog, SortDialogState, StatusBar, StyleState, Toggles, UNDO,
};
use ambre_trame::message::{
    CellFormat, CellStyle, HAlign, SheetName, SheetTab, TrameCommand, TrameEvent, VAlign,
};
use ambre_trame::ui::commands as registry;
use leptos::mount::mount_to;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

/// The events the chrome reads in these tests: TrameEvent documents, no channel.
type Event = UiEvent<TrameEvent>;

// Every test touches the DOM (or at least the leptos reactive graph backed by
// the browser), so run them inside headless Chrome rather than Node.
wasm_bindgen_test_configure!(run_in_browser);

/// Wrap a `StyleApplied` event the way the launcher relays it, so the mirror
/// under test sees exactly the shape it does in production.
fn style_applied(
    style: CellStyle,
    bold: ambre_core::Weight,
    italic: ambre_core::Slant,
    merged: ambre_trame::message::Merge,
    numeric: ambre_trame::message::CellValueKind,
) -> UiEvent<TrameEvent> {
    Event::Viewport {
        document_id: Id::<Document>::mint(),
        event: TrameEvent::StyleApplied {
            style,
            bold,
            italic,
            merged,
            numeric,
        },
    }
}

// -----------------------------------------------------------------------------
// Reactive / presentation belt
// -----------------------------------------------------------------------------

/// `apply_style_event` fans a `StyleApplied` out into every `StyleState` signal
/// the formatting toolbar subscribes to. This is the single style event, so
/// bold/italic/merged (reported beside the style) land here too.
#[wasm_bindgen_test]
fn style_applied_mirrors_into_style_state() {
    // A reactive owner is required to allocate signals outside a component.
    let _owner = OwnerScope::new();

    let state = StyleState::new();
    let style = CellStyle {
        underline: ambre_core::Underline::Underlined,
        wrap: ambre_core::WordBreak::Anywhere,
        h_align: HAlign::Right,
        v_align: VAlign::Bottom,
        format: CellFormat::Percent,
        ..CellStyle::default()
    };
    chrome::apply_style_event(
        &state,
        &style_applied(
            style,
            ambre_core::Weight::Bold,
            ambre_core::Slant::Upright,
            ambre_trame::message::Merge::Merged,
            ambre_trame::message::CellValueKind::Number,
        ),
    );

    assert_eq!(
        state.bold.get(),
        ambre_core::Weight::Bold,
        "bold reported separately must be mirrored"
    );
    assert_eq!(state.italic.get(), ambre_core::Slant::Upright);
    assert_eq!(
        state.merged.get(),
        ambre_trame::message::Merge::Merged,
        "merge pressed-state drives the merge toggle"
    );
    assert_eq!(state.underline.get(), ambre_core::Underline::Underlined);
    assert_eq!(state.wrap.get(), ambre_core::WordBreak::Anywhere);
    assert_eq!(
        state.numeric.get(),
        ambre_trame::message::CellValueKind::Number,
        "numeric gates Auto-alignment resolution"
    );
    assert_eq!(state.h_align.get(), HAlign::Right);
    assert_eq!(state.v_align.get(), VAlign::Bottom);
    assert_eq!(state.format.get(), CellFormat::Percent);
}

/// The replacement is document content, so the Debug form of a replace
/// request holds its length alone.
#[wasm_bindgen_test]
fn a_replace_request_prints_the_replacement_length_alone() {
    let text = ambre_bus::app::find::FindText::new("q".to_owned());
    let request = chrome::ReplaceRequest {
        query: FindQuery::new(text.expect("a test query is not empty"), Default::default()),
        replacement: "salary".to_owned(),
    };
    let printed = format!("{request:?}");
    assert!(!printed.contains("salary"), "{printed}");
    assert!(printed.contains("replacement_chars: 6"), "{printed}");
}

/// Any non-`StyleApplied` event leaves the `StyleState` untouched. The launcher
/// calls the mirror for every relayed event, so it must be a strict no-op for
/// the rest.
#[wasm_bindgen_test]
fn style_state_ignores_unrelated_events() {
    let _owner = OwnerScope::new();

    let state = StyleState::new();
    state.bold.set(ambre_core::Weight::Bold);
    let ev = Event::Viewport {
        document_id: Id::<Document>::mint(),
        event: TrameEvent::DataReset,
    };
    chrome::apply_style_event(&state, &ev);
    assert_eq!(
        state.bold.get(),
        ambre_core::Weight::Bold,
        "DataReset must not clobber the style state"
    );
}

/// `mirror_event` projects the authoritative Undo/Redo availability from
/// `TrameEvent::UndoState` into the command store the toolbar buttons read for
/// their `disabled` state.
#[wasm_bindgen_test]
fn undo_state_mirrors_into_command_store() {
    let _owner = OwnerScope::new();

    let store = CommandStates::new(chrome::COMMANDS.iter().map(|c| c.id));
    chrome::init_command_states(&store);
    // A fresh document starts with an empty history.
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
        event: TrameEvent::UndoState {
            undo: ambre_core::Availability::Enabled,
            redo: ambre_core::Availability::Disabled,
        },
    };
    chrome::mirror_event(&store, &ev);
    assert_eq!(
        store.enabled_signal(UNDO).unwrap().get(),
        ambre_chrome::Availability::Enabled
    );
    assert_eq!(
        store.enabled_signal(REDO).unwrap().get(),
        ambre_chrome::Availability::Disabled
    );
}

/// The `StaticKey<Commands> -> AppCommand` mapping: structural intents become a concrete
/// `TrameCommand`, while UI-only intents (style toggles the launcher intercepts)
/// map to `None`.
#[wasm_bindgen_test]
fn command_to_app_maps_intents() {
    assert!(matches!(
        chrome::command_to_app(UNDO),
        Some(TrameCommand::Undo)
    ));
    assert!(matches!(
        chrome::command_to_app(REDO),
        Some(TrameCommand::Redo)
    ));
    // BOLD is intercepted into a SetCellStyle patch by the launcher; no direct
    // AppCommand here.
    assert!(chrome::command_to_app(BOLD).is_none());
    // "Sort range..." is a UI-only intent the launcher intercepts to open the
    // SortDialog (the dialog emits SortSelectionMulti itself); no direct mapping.
    assert!(chrome::command_to_app(chrome::SORT_RANGE_MULTI).is_none());
}

// -----------------------------------------------------------------------------
// DOM belt
// -----------------------------------------------------------------------------

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `view` into a fresh `<div>` appended to the body and return it.
///
/// DOM tests must clean up after themselves, but they must NOT wipe the whole
/// body. The `wasm-bindgen-test` browser harness keeps its own
/// `<pre id="output">` / `<pre id="console_output">` elements there. It routes
/// every `console.log` through them, including the final "test result: ..."
/// line the runner polls for.
///
/// Clearing the body (`set_inner_html("")`) deletes those elements. The next
/// log then throws `appendChild` on null, the summary is never written, and the
/// runner times out (SIGKILL). Scoping each test to its own container --
/// removed via `container.remove()` -- leaves the harness elements untouched.
/// `mount_to(..).forget()` keeps the view mounted for the assertions (like
/// `mount_to_body`); the container removal tears it down.
/// A fresh reactive owner, current until the guard drops.
///
/// `Owner::set` alone leaves the thread-local pointing at the owner after the
/// test drops it, so the next test starts from a dead current owner. The
/// guard unsets it, then drops it, which cleans up every view mounted under it.
struct OwnerScope(Owner);

impl OwnerScope {
    fn new() -> Self {
        let owner = Owner::new();
        owner.set();
        Self(owner)
    }
}

impl Drop for OwnerScope {
    fn drop(&mut self) {
        self.0.clone().unset();
    }
}

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
            count: ambre_core::Count::<ambre_trame::message::Cells>::new(3),
            numbers: totals(&[1.0, 2.0, 3.0]),
        }));
        let perf = RwSignal::new(PerfSnapshot::default());
        let zoom = RwSignal::new(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE);
        let locale = RwSignal::new(None);
        view! { <StatusBar stats=stats perf=perf zoom=zoom locale=locale /> }
    });

    let text = container.text_content().unwrap_or_default();
    assert!(text.contains("Trame"), "status label missing: {text}");
    assert!(text.contains("Sum: 6"), "sum aggregate missing: {text}");
    assert!(text.contains("Count: 3"), "count aggregate missing: {text}");

    container.remove();
}

/// The find counter is built from the reply's zero-based index, and an index
/// past the matches focuses nothing (`UI-THREAD-060`).
#[wasm_bindgen_test]
fn a_find_position_counts_from_one_and_never_past_the_matches() {
    assert_eq!(
        chrome::FindPosition::new(Some(Pos::<Match>::new(0)), Len::<Match>::new(3)).label(),
        "1/3"
    );
    assert_eq!(
        chrome::FindPosition::new(Some(Pos::<Match>::new(2)), Len::<Match>::new(3)).label(),
        "3/3"
    );
    assert_eq!(
        chrome::FindPosition::new(Some(Pos::<Match>::new(3)), Len::<Match>::new(3)).label(),
        "0/3"
    );
    assert_eq!(
        chrome::FindPosition::new(None, Len::<Match>::new(0)).label(),
        "0/0"
    );
}

/// DOM test: the `FindReplace` panel mounts hidden when closed and, once opened
/// in find mode, exposes the "Find" input without the replace-only field.
#[wasm_bindgen_test]
fn find_replace_panel_opens_in_find_mode() {
    let doc = document();
    let container = mount_in_container(|| {
        let state = RwSignal::new(None::<FindReplaceState>);
        let result = RwSignal::new(None::<chrome::FindPosition>);
        let noop_find = Callback::new(|_: Option<FindQuery<ambre_trame::message::FindOptions>>| {});
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
            r1: Pos::<Row>::new(0),
            c1: Pos::<Col>::new(0),
            r2: Pos::<Row>::new(4),
            c2: Pos::<Col>::new(2),
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
    assert_eq!(
        req.header,
        ambre_trame::message::HeaderRow::Present,
        "header toggle carried into the request"
    );
    // Two distinct levels (the add-level default picks the next unused column).
    assert_eq!(req.sort_keys.len(), 2, "both levels emitted");
    assert_ne!(
        req.sort_keys[0].col, req.sort_keys[1].col,
        "distinct columns: a repeated column is dropped"
    );

    container.remove();
}

// -----------------------------------------------------------------------------
// Context-menu scope (`trame-context-menu`): the grouped/iconed entries and the
// header-scoped layout that drops the cell-only clipboard + merge groups.
// -----------------------------------------------------------------------------

/// Collect the command labels of an entry list, skipping separators.
fn item_commands(
    entries: &[ambre_chrome::MenuEntry],
) -> Vec<ambre_core::StaticKey<ambre_core::Commands>> {
    entries
        .iter()
        .filter_map(|e| match e {
            ambre_chrome::MenuEntry::Item(item) => Some(item.command()),
            ambre_chrome::MenuEntry::Separator => None,
        })
        .collect()
}

#[wasm_bindgen_test]
fn cell_menu_has_clipboard_and_merge() {
    let entries =
        chrome::context_entries_for(chrome::CtxScope::Cell, chrome::SheetFilter::Unfiltered);
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
    let entries =
        chrome::context_entries_for(chrome::CtxScope::Column, chrome::SheetFilter::Unfiltered);
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
    let entries =
        chrome::context_entries_for(chrome::CtxScope::Row, chrome::SheetFilter::Unfiltered);
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
    let entries =
        chrome::context_entries_for(chrome::CtxScope::Corner, chrome::SheetFilter::Unfiltered);
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
    let none = item_commands(&chrome::context_entries_for(
        chrome::CtxScope::Cell,
        chrome::SheetFilter::Unfiltered,
    ));
    assert!(none.contains(&chrome::FILTER_COLUMN));
    assert!(!none.contains(&chrome::CLEAR_FILTER));
    // Active filter -> "Remove filter" offered, "Filter by column" absent.
    let active = item_commands(&chrome::context_entries_for(
        chrome::CtxScope::Cell,
        chrome::SheetFilter::Filtered,
    ));
    assert!(active.contains(&chrome::CLEAR_FILTER));
    assert!(!active.contains(&chrome::FILTER_COLUMN));
}

/// DOM test for the `SheetTabs` strip. It renders one `.tab` per sheet with the
/// active one flagged. A click on another tab emits `on_activate`, and the
/// trailing "+" emits `on_create`. Guards the canvas->DOM migration of the tab
/// strip.
#[wasm_bindgen_test]
fn sheet_tabs_render_active_and_emit_intents() {
    let activated = RwSignal::new(None::<Id<Sheet>>);
    let created = RwSignal::new(0_u32);

    let container = mount_in_container(move || {
        let tab = |id: &str, name: &str| SheetTab {
            id: Id::<Sheet>::from_text(id.to_owned()),
            name: SheetName::new(name).unwrap(),
        };
        let sheets = RwSignal::new(vec![tab("s0", "One"), tab("s1", "Two")]);
        let active = RwSignal::new(Id::<Sheet>::from_text("s1".to_owned()));
        view! {
            <SheetTabs
                sheets=sheets
                active=active
                on_activate=Callback::new(move |id| activated.set(Some(id)))
                on_create=Callback::new(move |()| created.update(|n| *n += 1))
                on_rename=Callback::new(move |_: (Id<Sheet>, SheetName)| {})
                on_delete=Callback::new(move |_: Id<Sheet>| {})
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
    assert_eq!(
        activated.get_untracked(),
        Some(Id::<Sheet>::from_text("s0".to_owned()))
    );

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

/// A rename to a name the rule refuses keeps the editor open, its reason on
/// the field, and sends nothing. A valid name is sent (`TRAME-234`).
#[wasm_bindgen_test]
async fn sheet_tabs_refuse_a_name_the_rule_refuses() {
    let renamed = RwSignal::new(None::<(Id<Sheet>, SheetName)>);
    let container = mount_in_container(move || {
        let sheets = RwSignal::new(vec![SheetTab {
            id: Id::<Sheet>::from_text("s0".to_owned()),
            name: SheetName::new("One").unwrap(),
        }]);
        view! {
            <SheetTabs
                sheets=sheets
                active=RwSignal::new(Id::<Sheet>::from_text("s0".to_owned()))
                on_activate=Callback::new(move |_: Id<Sheet>| {})
                on_create=Callback::new(move |()| {})
                on_rename=Callback::new(move |pair| renamed.set(Some(pair)))
                on_delete=Callback::new(move |_: Id<Sheet>| {})
                on_reorder=Callback::new(move |_: ReorderSheet| {})
            />
        }
    });
    let tab: web_sys::HtmlElement = container
        .query_selector(".tab")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    // The chrome delegates its listeners to the document, so each event
    // bubbles.
    let dbl = web_sys::Event::new("dblclick").unwrap();
    dbl.init_event_with_bubbles("dblclick", true);
    tab.dispatch_event(&dbl).unwrap();
    // The double click swaps the tab for the editor on a scheduled effect.
    settle().await;
    let field = || -> web_sys::HtmlInputElement {
        container
            .query_selector(".tab-rename")
            .unwrap()
            .expect("the rename editor is open")
            .dyn_into()
            .unwrap()
    };
    let enter = || {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("Enter");
        init.set_bubbles(true);
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap()
    };

    field().set_value("Q1:Q2");
    field().dispatch_event(&enter()).unwrap();
    assert_eq!(renamed.get_untracked(), None, "a refused name is not sent");
    assert_eq!(
        field().get_attribute("aria-invalid").as_deref(),
        Some("true")
    );

    field().set_value("Data");
    field().dispatch_event(&enter()).unwrap();
    assert_eq!(
        renamed.get_untracked(),
        Some((
            Id::<Sheet>::from_text("s0".to_owned()),
            SheetName::new("Data").unwrap()
        ))
    );
    container.remove();
}

/// DOM test: every command button of the `FormatBar` announces the label the
/// registry declares (`ambre-spec-commands.md`, UI-THREAD-036).
///
/// This is the belt over the toolbar, the surface that repeated "Copy", "Bold"
/// and "Undo" as literals beside the table that declares them. It reads the
/// rendered `aria-label` out of the DOM, so a literal written back into the
/// markup fails here rather than drifting in silence.
#[wasm_bindgen_test]
fn the_toolbar_announces_the_registry_labels() {
    let container = mount_in_container(|| {
        let store = CommandStates::new(chrome::COMMANDS.iter().map(|c| c.id));
        chrome::init_command_states(&store);
        let toggles = Toggles::from_store(&store);
        let style = StyleState::new();
        let theme = RwSignal::new(ambre_canvas::Theme::Dark);
        view! {
            <FormatBar
                toggles=toggles
                style=style
                theme=theme
                on_command=Callback::new(|_| {})
                on_style=Callback::new(|_| {})
            />
        }
    });

    for id in [
        COPY, CUT, PASTE, FIND, IMPORT_CSV, EXPORT_CSV, SAVE_FILE, BOLD, ITALIC, MERGE, UNDO, REDO,
    ] {
        let expected = registry::title(id, &Locale::En).expect("described");
        let found = container
            .query_selector(&format!("[aria-label=\"{expected}\"]"))
            .unwrap();
        assert!(found.is_some(), "{id:?}: no control announces {expected:?}");
    }

    container.remove();
}

/// A peer's delete of the cell under an open session reaches the user as a
/// toast, not the log alone (`TRAME-UI-176`).
#[wasm_bindgen_test]
fn a_lost_edit_reaches_the_user() {
    let _owner = OwnerScope::new();
    let toasts = ambre_chrome::Toasts::new();
    chrome::notify_event(
        &toasts,
        &Event::Viewport {
            document_id: Id::<Document>::mint(),
            event: TrameEvent::EditTargetGone {
                session: ambre_bus::Token::FIRST,
            },
        },
    );
    let root = mount_in_container(move || view! { <ambre_chrome::ToastHost toasts=toasts /> });
    let text = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the loss is on screen, not in the log")
        .text_content()
        .unwrap_or_default();
    assert!(text.contains("deleted the cell you were editing"), "{text}");
    root.remove();
}

/// `INTEROP-007`: what an import carried and left out reaches the user as the
/// shared sentence, on screen and not in the log alone.
#[wasm_bindgen_test]
fn an_interop_report_reaches_the_user() {
    use ambre_bus::interop::{Direction, FileFormat, InteropAnswer, Occurrences, Outcome};
    use ambre_trame::message::{TrameConstruct, TrameReport};
    let _owner = OwnerScope::new();
    let toasts = ambre_chrome::Toasts::new();
    let mut report = TrameReport::new(Direction::Import, FileFormat::Csv);
    report.record_count(
        TrameConstruct::Row,
        Outcome::Carried,
        ambre_core::Count::<Occurrences>::new(2),
    );
    report.record_count(
        TrameConstruct::Column,
        Outcome::Carried,
        ambre_core::Count::<Occurrences>::new(2),
    );
    report.record(TrameConstruct::UnreadableRecord, Outcome::Dropped);
    chrome::notify_event(
        &toasts,
        &Event::Viewport {
            document_id: Id::<Document>::mint(),
            event: TrameEvent::Interop(InteropAnswer::ImportCompleted { report }),
        },
    );
    let root = mount_in_container(move || view! { <ambre_chrome::ToastHost toasts=toasts /> });
    let text = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the report is on screen")
        .text_content()
        .unwrap_or_default();
    assert!(text.contains("Imported 2 rows and 2 columns"), "{text}");
    assert!(text.contains("unreadable record"), "{text}");
    root.remove();
}

/// The totals of `values`, each a finite cell number. Their plain sum stands
/// for the Logic thread's exact one: the band shows what it is given.
fn totals(values: &[f64]) -> Option<ambre_trame::message::NumericStats> {
    use ambre_core::Bounded;
    use ambre_trame::message::{CellFloat, NumericStats};
    let numbers: Vec<Bounded<CellFloat>> = values
        .iter()
        .map(|&v| Bounded::<CellFloat>::new(v).unwrap())
        .collect();
    NumericStats::of_all(&numbers, Bounded::<CellFloat>::new(values.iter().sum()))
}

/// `LOCALE-021`: a status band in another locale than the workbook's says
/// so, and one in the workbook's says nothing.
#[wasm_bindgen_test]
fn the_status_bar_says_when_it_shows_another_locale() {
    let notice = |convention| {
        let container = mount_in_container(move || {
            let stats = RwSignal::new(None);
            let perf = RwSignal::new(PerfSnapshot::default());
            let zoom = RwSignal::new(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE);
            let locale = RwSignal::new(Some(ambre_trame::ui::locale::LocaleView {
                document: ambre_client_core::DocumentLocale::parse("en").unwrap(),
                reader: ambre_client_core::ReaderLocale {
                    own: ambre_client_core::DocumentLocale::parse("fr").unwrap(),
                    convention,
                },
            }));
            view! { <StatusBar stats=stats perf=perf zoom=zoom locale=locale /> }
        });
        let text = container
            .query_selector(".statusbar-locale")
            .unwrap()
            .and_then(|node| node.text_content());
        container.remove();
        text
    };
    assert_eq!(
        notice(ambre_client_core::NumberConvention::AllMine).as_deref(),
        Some("Numbers shown in fr; the document is in en")
    );
    assert_eq!(notice(ambre_client_core::NumberConvention::TypeMine), None);
}

/// Mount the status band over `stats` and return its text.
fn status_text(stats: Option<SelStats>) -> String {
    let container = mount_in_container(move || {
        let stats = RwSignal::new(stats);
        let perf = RwSignal::new(PerfSnapshot::default());
        let zoom = RwSignal::new(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE);
        let locale = RwSignal::new(None);
        view! { <StatusBar stats=stats perf=perf zoom=zoom locale=locale /> }
    });
    let text = container
        .query_selector(".statusbar-stats")
        .unwrap()
        .expect("the aggregates band is mounted")
        .text_content()
        .unwrap_or_default();
    container.remove();
    text
}

/// A selection with no number shows its count and no numeric aggregate
/// (`TRAME-UI-107`).
#[wasm_bindgen_test]
fn a_selection_with_no_number_shows_no_numeric_aggregate() {
    let text = status_text(Some(SelStats {
        count: ambre_core::Count::<ambre_trame::message::Cells>::new(4),
        ..SelStats::default()
    }));
    assert!(text.contains("Count: 4"), "{text}");
    for absent in ["Sum", "Avg", "Min", "Max"] {
        assert!(!text.contains(absent), "{absent} in {text}");
    }
    let text = status_text(Some(SelStats {
        count: ambre_core::Count::<ambre_trame::message::Cells>::new(4),
        numbers: totals(&[5.0]),
    }));
    for present in ["Sum: 5", "Avg: 5", "Min: 5", "Max: 5", "Count: 4"] {
        assert!(text.contains(present), "{present} not in {text}");
    }
    assert_eq!(status_text(None), "", "a cleared band shows nothing");
}

/// A whole aggregate reads with no decimals, any other with two
/// (`TRAME-UI-135`).
#[wasm_bindgen_test]
fn an_aggregate_reads_whole_or_with_two_decimals() {
    let text = status_text(Some(SelStats {
        count: ambre_core::Count::<ambre_trame::message::Cells>::new(3),
        numbers: totals(&[-2.0, 1.0 / 3.0, 9.5 - 1.0 / 3.0]),
    }));
    for present in ["Sum: 7.50", "Avg: 2.50", "Min: -2", "Max: 9.17"] {
        assert!(text.contains(present), "{present} not in {text}");
    }
    assert!(!text.contains("-2.00"), "{text}");
}

/// A sum past the largest finite number reads `#NUM!`, and so does its
/// average, while the extremes read as numbers (`TRAME-UI-179`).
#[wasm_bindgen_test]
fn a_sum_past_the_largest_number_reads_num() {
    let text = status_text(Some(SelStats {
        count: ambre_core::Count::<ambre_trame::message::Cells>::new(2),
        numbers: totals(&[f64::MAX, f64::MAX]),
    }));
    for present in ["Sum: #NUM!", "Avg: #NUM!", "Count: 2"] {
        assert!(text.contains(present), "{present} not in {text}");
    }
    for absent in ["inf", "NaN"] {
        assert!(!text.contains(absent), "{absent} in {text}");
    }
}

/// A refusal from the Logic thread reaches the user as a toast
/// (`TRAME-UI-120`): a refused merge and a refused command each say why.
#[wasm_bindgen_test]
fn a_refusal_reaches_the_user_as_a_toast() {
    let _owner = OwnerScope::new();
    let toasts = ambre_chrome::Toasts::new();
    for event in [
        TrameEvent::MergeRejected {
            at: ambre_trame::CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(1)),
        },
        TrameEvent::Error {
            refusal: ambre_trame::message::TrameRefusal::LastSheet,
        },
    ] {
        chrome::notify_event(
            &toasts,
            &Event::Viewport {
                document_id: Id::<Document>::mint(),
                event,
            },
        );
    }
    let root = mount_in_container(move || view! { <ambre_chrome::ToastHost toasts=toasts /> });
    let toasts = root.query_selector_all(".ambre-toast").unwrap();
    let text: Vec<String> = (0..toasts.length())
        .filter_map(|i| toasts.item(i)?.text_content())
        .collect();
    assert_eq!(text.len(), 2, "{text:?}");
    assert!(text[0].contains("cell B1 is not empty"), "{text:?}");
    assert!(text[1].contains("at least one sheet"), "{text:?}");
    root.remove();
}

/// Let the effects a click scheduled run before the DOM is read. Wait for the
/// browser tasks they queue too, such as the `close` event of a dialog.
async fn settle() {
    for _ in 0..2 {
        let wait = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
        });
        wasm_bindgen_futures::JsFuture::from(wait).await.unwrap();
    }
}

/// A relay over fresh signals, as the shell builds it.
fn relay() -> chrome::TrameRelay {
    chrome::TrameRelay {
        sort_dialog: RwSignal::new(None),
        filter_popover: RwSignal::new(None),
        filter_values: RwSignal::new(Vec::new()),
        sheets: RwSignal::new(Vec::new()),
        sel_stats: RwSignal::new(None),
        locale: RwSignal::new(None),
    }
}

fn button(root: &web_sys::Element, selector: &str) -> web_sys::HtmlButtonElement {
    root.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("{selector}"))
        .dyn_into()
        .unwrap()
}

/// A sort dialog over `state`, recording what it emits.
fn sort_dialog(
    state: RwSignal<Option<SortDialogState>>,
) -> (web_sys::Element, RwSignal<Option<SortApply>>) {
    let emitted = RwSignal::new(None::<SortApply>);
    let on_apply = Callback::new(move |req: SortApply| emitted.set(Some(req)));
    let root = mount_in_container(move || view! { <SortDialog state=state on_apply=on_apply /> });
    (root, emitted)
}

/// `TRAME-FEAT-046`: the toast a filter raises says how many rows it hid.
#[wasm_bindgen_test]
fn the_filter_toast_says_how_many_rows_it_hid() {
    let _owner = OwnerScope::new();
    let toasts = ambre_chrome::Toasts::new();
    chrome::notify_event(
        &toasts,
        &Event::Viewport {
            document_id: Id::<Document>::mint(),
            event: TrameEvent::FilterApplied {
                hidden_row_count: Len::<Row>::new(37),
                col: Pos::<Col>::new(2),
            },
        },
    );
    let root = mount_in_container(move || view! { <ambre_chrome::ToastHost toasts=toasts /> });
    let toast = root
        .query_selector(".ambre-toast")
        .unwrap()
        .and_then(|t| t.text_content())
        .expect("a toast");
    assert!(toast.contains("37"), "{toast}");
    root.remove();
}

/// `TRAME-UI-101`: the reply to a sort request opens the dialog, and it
/// offers exactly the columns of that range, D to F.
#[wasm_bindgen_test]
fn the_sort_reply_opens_the_dialog_over_its_columns() {
    let _owner = OwnerScope::new();
    let relay = relay();
    relay.absorb(&Event::Viewport {
        document_id: Id::<Document>::mint(),
        event: TrameEvent::SortRange {
            r1: Pos::<Row>::new(2),
            c1: Pos::<Col>::new(3),
            r2: Pos::<Row>::new(9),
            c2: Pos::<Col>::new(5),
        },
    });
    let (root, _) = sort_dialog(relay.sort_dialog);
    let options = root.query_selector_all(".ambre-sort-col option").unwrap();
    let labels: Vec<String> = (0..options.length())
        .filter_map(|i| options.item(i)?.text_content())
        .collect();
    assert_eq!(labels, ["Column D", "Column E", "Column F"]);
    root.remove();
}

/// `TRAME-UI-150`: the dialog holds one level at least and one per column at
/// most. Both bounds are reached and both buttons refuse past them.
#[wasm_bindgen_test]
async fn the_dialog_holds_one_level_to_one_per_column() {
    let state = RwSignal::new(Some(SortDialogState {
        r1: Pos::<Row>::new(0),
        c1: Pos::<Col>::new(0),
        r2: Pos::<Row>::new(4),
        c2: Pos::<Col>::new(2),
    }));
    let (root, _) = sort_dialog(state);
    let levels = || {
        root.query_selector_all(".ambre-sort-level")
            .unwrap()
            .length()
    };
    settle().await;
    assert_eq!(levels(), 1);
    assert!(button(&root, ".ambre-sort-remove").disabled(), "the floor");
    for _ in 0..2 {
        button(&root, ".ambre-sort-add").click();
        settle().await;
    }
    assert_eq!(levels(), 3);
    assert!(button(&root, ".ambre-sort-add").disabled(), "the ceiling");
    assert!(!button(&root, ".ambre-sort-remove").disabled());
    for _ in 0..2 {
        button(&root, ".ambre-sort-remove").click();
        settle().await;
    }
    assert_eq!(levels(), 1);
    assert!(button(&root, ".ambre-sort-remove").disabled(), "the floor");
    root.remove();
    // One column: the ceiling is the floor.
    let state = RwSignal::new(Some(SortDialogState {
        r1: Pos::<Row>::new(0),
        c1: Pos::<Col>::new(4),
        r2: Pos::<Row>::new(9),
        c2: Pos::<Col>::new(4),
    }));
    let (root, _) = sort_dialog(state);
    settle().await;
    assert!(button(&root, ".ambre-sort-add").disabled());
    root.remove();
}

/// `TRAME-UI-151`: the open dialog is modal, so the sheet behind it takes no
/// pointer. The keys are held by the shell's key listener.
#[wasm_bindgen_test]
async fn the_open_sort_dialog_is_modal() {
    let state = RwSignal::new(Some(SortDialogState {
        r1: Pos::<Row>::new(0),
        c1: Pos::<Col>::new(0),
        r2: Pos::<Row>::new(4),
        c2: Pos::<Col>::new(1),
    }));
    let (root, _) = sort_dialog(state);
    settle().await;
    let dialog = root
        .query_selector("dialog.ambre-sort-modal")
        .unwrap()
        .unwrap();
    assert!(dialog.matches(":modal").unwrap());
    root.remove();
}

/// `TRAME-UI-153`: cancel, the close button and the backdrop each close the
/// dialog and none of them sorts.
///
/// Each path ends in the native `close` event, which the browser queues as a
/// task of its own source. A timer tick does not order after it, so the test
/// awaits the event itself before it reads the snapshot.
#[wasm_bindgen_test]
async fn closing_the_sort_dialog_sorts_nothing() {
    for selector in [
        ".ambre-sort-cancel",
        ".ambre-sort-close",
        "dialog.ambre-sort-modal",
    ] {
        let state = RwSignal::new(Some(SortDialogState {
            r1: Pos::<Row>::new(0),
            c1: Pos::<Col>::new(0),
            r2: Pos::<Row>::new(4),
            c2: Pos::<Col>::new(2),
        }));
        let (root, emitted) = sort_dialog(state);
        settle().await;
        let dialog = root
            .query_selector("dialog.ambre-sort-modal")
            .unwrap()
            .unwrap();
        let closed = js_sys::Promise::new(&mut |resolve, _| {
            dialog
                .add_event_listener_with_callback("close", &resolve)
                .unwrap();
        });
        root.query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("{selector}"))
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        wasm_bindgen_futures::JsFuture::from(closed).await.unwrap();
        settle().await;
        assert_eq!(state.get_untracked(), None, "{selector}");
        assert_eq!(emitted.get_untracked(), None, "{selector}");
        root.remove();
    }
}
