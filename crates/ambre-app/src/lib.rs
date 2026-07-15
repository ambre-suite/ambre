// ambre-app/src/lib.rs
// ============================================================================
// Launcher -- the cdylib entry point (#[wasm_bindgen(start)]), running on the
// page's main JS thread (the UI thread).
//
// SPA model (ambre-spec-architecture): the ENGINE is created once and lives for
// the whole session -- the inter-thread channels, the Logic thread, the Sync
// worker, the local-persistence store, and the wgpu canvas. A `leptos_router`
// shell drives a CLIENT-side route (pushState, no full page load); navigating
// between the document list and a document only swaps the canvas's active
// document in place (`DocSwitcher`), so the Sync worker keeps its in-memory
// session keys across a navigation.
//
//   - UI (this main thread) + Logic (a `wasm_thread` worker) share this Main
//     wasm instance's memory and talk over `flume` channels.
//   - Sync is a SEPARATE wasm instance in its own Web Worker (own memory),
//     spawned here via `web_sys::Worker` and reached over `postMessage`. The
//     main thread relays Logic<->Sync traffic in the single per-frame relay.
//   - The Leptos shell mounts once: the AppShell chrome (which owns the
//     `<canvas>`) plus a landing overlay shown on the `/` route.
//   - One route-independent rAF loop (the canvas host's) runs the per-frame
//     relay AND the dispatcher, whatever the route -- paused (frozen frame)
//     while the landing overlay covers the canvas.
//
// Command model (ambre-spec-ui.md 2.C): every trigger -- toolbar button,
// keyboard shortcut, context menu -- produces the SAME `CommandId` ->
// `LogicCommand`, targeting the currently routed document (`current_doc`).
//
// Compiles ONLY on wasm32 targets (depends on web_sys, wgpu, leptos).
// ============================================================================

#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{
    AppCommand, AppEvent, DocumentId, LogicCommand, LogicEvent, MemberInfo, RequestId, RestCommand,
    Role, SyncCommand, TrameCommand, UiEvent,
};
use ambre_chrome::commands::{CommandId, Keymap};
use ambre_chrome::{
    AppShell, CommandStates, ContextMenu, Dialog, FormulaBar, Header, StatusBar, Tabs, ToastHost,
    Toasts, Toolbar,
};
// Trame-specific chrome (command wiring + toolbar / selection-toolbar markup +
// status label). The launcher stays app-agnostic and reaches into this module
// for every Trame concern -- it names no command, button, or event variant itself.
use ambre_toile::chrome as toile_chrome;
use ambre_toile::ui::ToileView;
use ambre_trame::chrome as trame_chrome;
use ambre_trame::render::FormulaAction;
use ambre_trame::ui::TrameView;
use leptos::prelude::*;
use leptos_icons::Icon;
use leptos_router::components::Router;
use leptos_router::hooks::use_location;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

mod host;
mod landing;
mod route;

// The crate's unit tests (pure helpers in `route`/`host`) run under headless
// Chrome via `wasm-pack test --headless --chrome` (`just test-wasm`). `--chrome`
// sets `WASM_BINDGEN_TEST_ONLY_WEB`, so the runner skips any test binary not
// configured for the browser -- declare it once here for the whole lib-test.
#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use host::{AppView, DocSwitcher};

use landing::{DocItem, LandingSignals};

/// Capacity of every inter-thread channel.
///
/// Deliberately generous: P3 traffic is request/response and never approaches
/// it, and the hot viewport path is coalesced to <=1 send/frame on the UI side
/// (see `TrameView`). It exists purely as a safety bound so a momentarily-slow
/// consumer cannot grow an unbounded queue in the shared heap.
const CHANNEL_CAPACITY: usize = 1024;

/// The handles the Leptos shell needs. Every field is `Send` (a `flume::Sender`,
/// an `Arc`-backed control, or a Copy `RwSignal`): `leptos_router`'s `<Router>`
/// requires its subtree be `Send`, so the `!Send` `Rc`-backed handles (the
/// `CommandStates` store, the `DocSwitcher`) are kept OUT of the view -- the
/// store is driven by the per-frame relay and the canvas swap by the route poll,
/// both outside the reactive tree. The shell sees only the toggle signals the
/// store exposes.
#[derive(Clone)]
struct Wiring {
    to_logic: flume::Sender<LogicCommand>,
    raf: ambre_canvas::RafControl,
    theme_ctl: ambre_canvas::ThemeControl,
    /// The active app's command toggle signals (the store's reactive view,
    /// chrome side). Opaque to the launcher -- the app's chrome reads its own.
    toggles: trame_chrome::Toggles,
    /// The active cell's rich formatting, driving the formatting toolbar's
    /// font/size selects, colour pickers, alignment, number format, borders, wrap
    /// and underline controls (`-ui.md` sec.5bis). Fed by `TrameEvent::StyleApplied`.
    style_state: trame_chrome::StyleState,
    /// Floating selection-toolbar anchor (canvas-relative CSS px).
    anchor_pos: RwSignal<Option<(f64, f64)>>,
    /// Transient notifications (ARIA live region).
    toasts: Toasts,
    /// Signed-in user's display name; `None` = signed out.
    username: RwSignal<Option<String>>,
    /// The user's documents (landing list).
    docs: RwSignal<Vec<DocItem>>,
    /// Members of the active document (share dialog). Populated by the per-frame
    /// relay on `DocumentMembers`; empty until the dialog requests the list.
    members: RwSignal<Vec<MemberInfo>>,
    /// Whether the server enables dev login; `None` until `/api/auth/config` answers.
    dev_mode: RwSignal<Option<bool>>,
    /// The currently routed document, or `None` on the landing route. Read by
    /// every command trigger so it targets the active document.
    current_doc: RwSignal<Option<DocumentId>>,
    /// Formula bar: active cell reference label (e.g. "B2").
    formula_ref: RwSignal<String>,
    /// Formula bar: cell raw text / live edit buffer.
    formula_text: RwSignal<String>,
    /// Formula bar: commit/cancel action from the HTML input.
    formula_action: RwSignal<Option<FormulaAction>>,
    /// Shared filter popover open state (`-ui.md` sec.5.D); `None` = closed.
    filter_popover: RwSignal<Option<trame_chrome::FilterPopoverState>>,
    /// The active column's distinct visible values, feeding the popover's "by
    /// values" list. Set by the per-frame relay on `TrameEvent::ColumnValues`.
    filter_values: RwSignal<Vec<String>>,
    /// Find/replace panel open state (`-ui.md` sec.5.G); `None` = closed.
    find_state: RwSignal<Option<trame_chrome::FindReplaceState>>,
    /// Last `FindResult` summarized as `(active_1based, total)` for the panel's
    /// "k/n" counter. Set by the per-frame relay on `TrameEvent::FindResult`.
    find_result: RwSignal<Option<(usize, usize)>>,
    /// Whether the booted document is a Toile board. The chrome layer is still
    /// largely Trame-shaped; this gates the Toile-specific panels (the Find panel,
    /// `toile-ui §5quater`) which replace their Trame equivalents. Cross-type
    /// in-session navigation reloads the page, so it is fixed per page load.
    boot_is_toile: bool,
    /// Toile Find panel open flag (`toile-ui §5quater`); Ctrl+F sets it.
    find_state_toile: RwSignal<bool>,
    /// Last Toile `FindResults` (hits + focus), feeding the panel's counter and
    /// step origin. Set by the per-frame relay on `ToileEvent::FindResults`.
    find_result_toile: RwSignal<Option<ambre_bus::app::toile::FindResults>>,
    /// Presenter overlay panel open flag (`toile-features` §1.D). Driven by the
    /// relay on `ToileEvent::PresentModeChanged`; the panel is chrome, so its
    /// visibility lives here rather than canvas-side.
    present_toile: RwSignal<bool>,
    /// Active slide index / deck length feeding the presenter panel's `k / n`
    /// counter. Set by the relay on the enriched `ToileEvent::SlideChanged`.
    presenter_index: RwSignal<usize>,
    presenter_count: RwSignal<usize>,
    /// Active slide's speaker notes, re-seeded on each slide change; the panel
    /// editor diffs against it and emits `EditNotes`.
    presenter_notes: RwSignal<String>,
    /// Active slide's frame id: the target of the panel's `EditNotes` intent.
    /// `None` when the deck is empty or the slide is dangling (edits skipped).
    presenter_frame: RwSignal<Option<ambre_bus::app::toile::ElementId>>,
    /// "Follow presenter" role signals (`toile-features` §1.B). `broadcasting`
    /// mirrors the local broadcast toggle, `following` the active follow target,
    /// and `presenter_peer` the latest peer seen broadcasting (drives the panel's
    /// Follow affordance).
    broadcasting_toile: RwSignal<bool>,
    following_toile: RwSignal<Option<ambre_bus::UserId>>,
    presenter_peer_toile: RwSignal<Option<ambre_bus::UserId>>,
    /// Toile right-click context-menu scope (`toile-ui §5ter`): the canvas view
    /// publishes it on each right-click; the shell's entry `Memo` reads it to build
    /// the menu, omitting inapplicable rows.
    toile_ctx_scope: RwSignal<ambre_toile::ui::menu::CtxScope>,
    /// The Toile context-menu command the shell forwards back into the canvas view
    /// (`toile-ui §5ter`): the view drains it and resolves it against its selection
    /// (the Logic thread never sees the selection).
    toile_menu_cmd: RwSignal<Option<ambre_chrome::commands::CommandId>>,
    /// Toile active tool (`toile-ui §5quinquies.A`): the shared source of truth for
    /// the DOM tool palette and the bare-key tool chords; the canvas view reads it
    /// each frame and applies it. Tool selection is UI-only, canvas-local state.
    toile_active_tool: RwSignal<ambre_toile::ui::Tool>,
    /// Toile style-inspector model (`toile-ui §5quinquies.B`): the selection's
    /// common `ElementStyle` + mixed-field mask, published by the canvas view; the
    /// DOM inspector reads it to render values and mixed-state. `None` hides it.
    toile_style_model: RwSignal<Option<toile_chrome::StyleModel>>,
    /// Whole-style edit the inspector emits back (`toile-ui §5quinquies.B`): the
    /// canvas view drains it into one `RestyleElements` over the live selection.
    toile_restyle: RwSignal<Option<ambre_bus::app::toile::ElementStyle>>,
    /// Floating selection toolbar placement + flags (`toile-ui §5quinquies.D`),
    /// republished every frame by the canvas view so the bar tracks pan/zoom;
    /// `None` hides it.
    toile_float_bar: RwSignal<Option<toile_chrome::FloatBar>>,
    /// Quick-colour pick the floating bar emits back (`toile-ui §5quinquies.D`):
    /// the canvas view folds it into a fill-only `RestyleElements` over the live
    /// selection.
    toile_quick_fill: RwSignal<Option<toile_chrome::QuickFill>>,
    /// Status-bar selection aggregates (`-ui.md` sec.13bis); `None` on a single
    /// cell. Set by the per-frame relay on `TrameEvent::SelectionStats`, cleared
    /// by the canvas view on a single-cell selection.
    sel_stats: RwSignal<Option<trame_chrome::SelStats>>,
    /// Toile status-bar selection aggregates (toile-ui §8bis): element tally +
    /// combined world bounds; `None` on an empty selection. Set by the per-frame
    /// relay on `ToileEvent::SelectionStats`.
    sel_stats_toile: RwSignal<Option<toile_chrome::SelStats>>,
    /// Status-bar perf snapshot (fps + frame timings), pumped from the canvas
    /// view's `PerfPublisher` by the per-frame relay (~2 Hz).
    perf_snap: RwSignal<ambre_canvas::PerfSnapshot>,
    /// Grid zoom factor (`1.0` = 100%, `-ui.md` sec.13bis): the status-bar menu
    /// writes it, the canvas view polls it each frame. Local view state, not synced.
    zoom: RwSignal<f64>,
    /// Right-click context-menu scope (`-ui.md` sec.5.C): the canvas view sets it
    /// (cell vs header) before the DOM menu opens; the shell derives the layout.
    ctx_scope: RwSignal<trame_chrome::CtxScope>,
    /// Whether the active sheet currently has a shared filter (`-ui.md` sec.5.D):
    /// mirrored from `TrameEvent::FilterApplied`/`FilterCleared`. Drives the
    /// context menu's dynamic filter entry (Filter by column XOR Remove filter).
    filter_active: RwSignal<bool>,
    /// Current selection rectangle `(r1, c1, r2, c2)`, published by the canvas
    /// view on each selection change; read when opening the multi-column sort
    /// dialog so it offers the selection's columns (`-ui.md` sec.5.C).
    sel_bounds: RwSignal<Option<(u32, u32, u32, u32)>>,
    /// Multi-column sort dialog open state (`-ui.md` sec.5.C); `None` = closed.
    sort_dialog: RwSignal<Option<trame_chrome::SortDialogState>>,
    /// Sheet-tab strip model (`-ui.md` sec.13): `(id, name)` in display order.
    /// Mirrored from `TrameEvent::SheetList` by the per-frame relay; drives the
    /// DOM `SheetTabs` slot (the tabs no longer live on the canvas).
    sheets: RwSignal<Vec<(String, String)>>,
    /// Active sheet id, mirrored from `TrameEvent::SheetList`.
    active_sheet: RwSignal<String>,
    /// Chrome status indicator (`-chrome.md` 2.G): drives the header sync label
    /// and the document-loading overlay. Set by the per-frame relay on
    /// `UiEvent::DocumentReady` / `ConnectionStatus`, and by the route poll on a
    /// document switch (which arms `hydrating`).
    chrome_status: RwSignal<ChromeStatus>,
}

/// Header status + loading state, derived from two independent signals carried
/// in one value so the relay updates each field where it learns it.
///
/// - `hydrating`: true from opening a document until its initial snapshot lands
///   (`UiEvent::DocumentReady`). Drives the canvas loading overlay and the
///   transient "Syncing" label. An empty/brand-new document never produces a
///   snapshot, so this is cleared by the explicit ready event, not by content.
/// - `connected`: mirrors the document's WebSocket link
///   (`UiEvent::ConnectionStatus`). Drives the "Synced" vs "Offline" label.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct ChromeStatus {
    hydrating: bool,
    connected: bool,
}

impl ChromeStatus {
    /// Localized header label for the current state (`-chrome.md` 2.G). While the
    /// document is hydrating the link state is irrelevant -- the user reads it as
    /// "still loading".
    fn label(self) -> &'static str {
        match (self.hydrating, self.connected) {
            (true, _) => "Syncing",
            (false, true) => "Synced",
            (false, false) => "Offline",
        }
    }

    /// CSS modifier for the status dot colour (paired with `.dot` in styles.css).
    fn dot_class(self) -> &'static str {
        match (self.hydrating, self.connected) {
            (true, _) => "dot syncing",
            (false, true) => "dot",
            (false, false) => "dot offline",
        }
    }
}

// Gated out under `cfg(test)`: the wasm-bindgen-test harness generates its own
// entry point, which collides with this `#[wasm_bindgen(start)] fn main`
// ("entry symbol `main` declared multiple times"). The launcher boot path is not
// what the unit tests exercise (they cover the pure helpers -- route/base64/fps),
// so dropping `main` from the test build is harmless.
#[cfg(not(test))]
#[wasm_bindgen(start)]
pub fn main() {
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(if cfg!(debug_assertions) {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            })
            .build(),
    );
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        let msg = info.to_string();
        // Latch the canvas fatal screen. Works from any thread sharing this
        // instance's memory (UI/Logic): a Logic-thread panic has no `window`,
        // so the JS overlay below cannot fire, but the UI thread's rAF loop is
        // still alive and will paint the terminal screen from this latch.
        ambre_canvas::signal_fatal(&msg);
        // Notify the page that the WASM application has suffered an
        // unrecoverable panic (panic=abort, so no graceful shutdown).
        if let Some(window) = web_sys::window() {
            if let Ok(f) = js_sys::Reflect::get(&window, &"ambreFatal".into()) {
                if let Ok(f) = f.dyn_into::<js_sys::Function>() {
                    let _ = f.call1(&window, &msg.into());
                }
            }
        }
    }));

    // Inter-thread channels (all `flume`, in this Main instance's shared heap).
    //   UI    --LogicCommand--> Logic      Logic --UiEvent--> UI
    //   Logic --SyncCommand---> [main relay] --postMessage--> Sync worker
    //   Sync worker --postMessage--> [main relay] --LogicEvent--> Logic
    //
    // Bounded (backpressure). The UI runs on the page main thread and must never
    // block, so its sends use `try_send` and drop on `Full`. Logic blocks on a
    // `flume::Selector`. The Logic<->Sync halves on the main side (`logic_to_sync_rx`,
    // `sync_to_logic_tx`) are owned by the per-frame relay below, NOT by a worker.
    let (ui_to_logic_tx, ui_to_logic_rx) = flume::bounded::<LogicCommand>(CHANNEL_CAPACITY);
    let (logic_to_ui_tx, logic_to_ui_rx) = flume::bounded::<UiEvent>(CHANNEL_CAPACITY);
    let (logic_to_sync_tx, logic_to_sync_rx) = flume::bounded::<SyncCommand>(CHANNEL_CAPACITY);
    let (sync_to_logic_tx, sync_to_logic_rx) = flume::bounded::<LogicEvent>(CHANNEL_CAPACITY);
    // Logic -> Main, carrying full document snapshots for local persistence. The
    // Main thread owns the IndexedDB store (its callbacks need a live event loop,
    // which the blocking Logic loop cannot offer) and drains this in `pre_frame`.
    let (logic_to_persist_tx, logic_to_persist_rx) =
        flume::bounded::<(DocumentId, Vec<u8>)>(CHANNEL_CAPACITY);

    // Dispatcher -> canvas view: the content event stream, forwarded after the
    // dispatcher has mirrored command-state events into the chrome.
    let (view_tx, view_rx) = flume::bounded::<UiEvent>(CHANNEL_CAPACITY);

    // Command keymap (the keyboard view of the registry). The active app binds
    // its own chords -- the launcher names none. Populated once the boot document
    // type is resolved below (Toile binds a different, smaller set than Trame).
    let mut keymap = Keymap::new();

    // Handle to the canvas rAF loop, so a fullscreen modal -- or the landing
    // overlay covering the canvas -- can freeze it.
    let raf = ambre_canvas::RafControl::new();

    // Canvas->DOM selection anchor: the view publishes the active cell's screen
    // position, the dispatcher mirrors it into the chrome's `anchor_pos` signal,
    // and the floating selection toolbar transform-syncs onto it.
    let anchor_pub = ambre_canvas::AnchorPublisher::new();

    // Canvas->DOM perf channel: the view publishes the per-frame perf snapshot
    // (fps + timings), the dispatcher mirrors it into the `perf_snap` signal the
    // status bar renders. The HUD migrated off the wgpu overlay (`-ui.md`
    // sec.13bis) -- no point shaping debug glyphs on the GPU every frame.
    let perf_pub = ambre_canvas::PerfPublisher::new();

    // Canvas->DOM filter-open channel (`-ui.md` sec.5.D): the view raises this
    // one-shot intent when an in-canvas autofilter button is clicked; the
    // per-frame relay consumes it (after mirroring the anchor) and opens the DOM
    // FilterPopover through the same path as the context-menu "Filter by column"
    // entry, so there is one filter code path with two entry points.
    let filter_open_pub = ambre_canvas::FilterOpenPublisher::new();

    // Formula bar signals (HTML input -> TrameView sync). RwSignal is Copy, so
    // we create them once and pass clones to both TrameView and the Leptos shell.
    let formula_ref = RwSignal::new(String::new());
    let formula_text = RwSignal::new(String::new());
    let formula_action = RwSignal::new(None::<FormulaAction>);

    // Status-bar signals (`-ui.md` sec.13bis): selection aggregates (set from the
    // `SelectionStats` event / cleared by the view on a single cell) and the live
    // perf snapshot pumped from `perf_pub` below.
    let sel_stats = RwSignal::new(None::<trame_chrome::SelStats>);
    // Toile selection aggregates (toile-ui §8bis): element tally + world bounds,
    // mirrored from `ToileEvent::SelectionStats`. Cleared to `None` on an empty
    // selection so the band shows only the zoom chip.
    let sel_stats_toile = RwSignal::new(None::<toile_chrome::SelStats>);
    // Presenter overlay panel signals (toile-features §1.D), mirrored from the
    // enriched `SlideChanged` + `PresentModeChanged` events by the per-frame
    // relay. Created here (like `sel_stats_toile`) and passed to the shell.
    let present_toile = RwSignal::new(false);
    let presenter_index = RwSignal::new(0usize);
    let presenter_count = RwSignal::new(0usize);
    let presenter_notes = RwSignal::new(String::new());
    let presenter_frame = RwSignal::new(None::<ambre_bus::app::toile::ElementId>);
    // "Follow presenter" role signals (toile-features §1.B), mirrored from the
    // `PresentBroadcastChanged` / `FollowingChanged` events and from a peer's
    // broadcasting presence. `presenter_peer_toile` is the latest peer seen
    // broadcasting a slide; it drives the panel's Follow affordance.
    let broadcasting_toile = RwSignal::new(false);
    let following_toile = RwSignal::new(None::<ambre_bus::UserId>);
    let presenter_peer_toile = RwSignal::new(None::<ambre_bus::UserId>);
    let perf_snap = RwSignal::new(ambre_canvas::PerfSnapshot::default());

    // Sheet-tab strip model (`-ui.md` sec.13): mirrored from `TrameEvent::SheetList`
    // by the relay below and fed to the DOM `SheetTabs` slot (off the canvas now).
    let sheets = RwSignal::new(Vec::<(String, String)>::new());
    let active_sheet = RwSignal::new(String::new());
    // Grid zoom factor selected in the status-bar menu (`-ui.md` sec.13bis),
    // polled by the canvas view each frame. Defaults to 100%.
    let zoom = RwSignal::new(1.0_f64);

    // Context-menu scope (`-ui.md` sec.5.C): the view sets it on each right-click
    // (cell vs header) before the DOM `contextmenu` listener opens the menu; the
    // shell derives the menu layout from it. Created here (like the formula
    // signals) so both the view and the Leptos shell share the one signal.
    let ctx_scope = RwSignal::new(trame_chrome::CtxScope::Cell);
    // Active-filter flag (`-ui.md` sec.5.D), mirrored from the Logic thread's
    // FilterApplied/FilterCleared. Drives the context menu's dynamic filter entry.
    let filter_active = RwSignal::new(false);
    // Selection rectangle published by the canvas view (`-ui.md` sec.5.C), read
    // when opening the multi-column sort dialog. Created here (like `ctx_scope`)
    // so the view and the Leptos shell share the one signal.
    let sel_bounds = RwSignal::new(None::<(u32, u32, u32, u32)>);

    // Toile context-menu bridge (`toile-ui §5ter`): the canvas view publishes the
    // selection scope on each right-click; the DOM menu reads it to build its
    // entries and writes the chosen `CommandId` back for the view to resolve
    // against its (UI-only) selection. Created here so the view and the Leptos
    // shell share the one pair of signals, exactly like Trame's `ctx_scope`.
    let toile_ctx_scope = RwSignal::new(ambre_toile::ui::menu::CtxScope::default());
    let toile_menu_cmd = RwSignal::new(None::<ambre_chrome::commands::CommandId>);
    // Toile active tool (`toile-ui §5quinquies.A`): shared by the DOM tool palette,
    // the keyboard tool chords and the canvas view, exactly like the context-menu
    // pair above. Starts on the default tool (Select).
    let toile_active_tool = RwSignal::new(ambre_toile::ui::Tool::default());
    // Toile style inspector (`toile-ui §5quinquies.B`): the view publishes the
    // selection's common style into `toile_style_model` and drains a whole-style
    // edit from `toile_restyle`, exactly like the context-menu pair above.
    let toile_style_model = RwSignal::new(None::<toile_chrome::StyleModel>);
    let toile_restyle = RwSignal::new(None::<ambre_bus::app::toile::ElementStyle>);
    // Floating selection toolbar (`toile-ui §5quinquies.D`): the view republishes
    // its placement into `toile_float_bar` each frame and drains a quick-colour
    // pick from `toile_quick_fill`, mirroring the inspector's model / restyle pair.
    let toile_float_bar = RwSignal::new(None::<toile_chrome::FloatBar>);
    let toile_quick_fill = RwSignal::new(None::<toile_chrome::QuickFill>);

    // Design tokens: inject the generated CSS custom properties (theme.css) and
    // boot the dark theme. `theme_ctl` is the single source of truth for the
    // active scheme, shared with the chrome toggle and the canvas view.
    let theme_ctl = ambre_canvas::ThemeControl::new(ambre_canvas::Theme::Dark);
    inject_theme(theme_ctl.get());

    // Resolve the boot route once. `/` is the landing (document list); a
    // `/trame/<id>` deep-link boots straight onto that document. After boot the
    // route is a CLIENT concern (leptos_router), no longer re-read here.
    let boot_route = route::current();
    // The routed document with its type (`/trame/<id>` -> (Trame, id)), kept for
    // the local restore which must materialise the doc with the right type if it
    // wins the startup race against `OpenDocument`.
    let boot_doc_typed = boot_route.doc();
    let boot_doc = boot_doc_typed.map(|(_, id)| id);
    // Cross-type in-session navigation reloads the page (the `DocSwitcher` only
    // swaps the id within one `AppView`), so the booted app kind is fixed here and
    // gates both the render view (below) and the app-specific chrome (Find panel,
    // keymap). Toile binds its own, smaller chord set.
    let boot_is_toile = matches!(boot_doc_typed, Some((ambre_bus::DocType::Toile, _)));
    if boot_is_toile {
        toile_chrome::bind_keys(&mut keymap);
    } else {
        trame_chrome::bind_keys(&mut keymap);
    }
    // The canvas needs a document from frame 0. Boot onto the routed one, or a
    // throwaway id on the landing route (the canvas is hidden there; the route
    // effect swaps it to the real document on the first navigation).
    let initial_doc = boot_doc.unwrap_or_else(DocumentId::new);

    // Spawn the Logic thread (shares this instance's memory). Owns the UI/Sync
    // halves of its channels; the Sync-bound half (`logic_to_sync_tx`) is relayed
    // to the Sync worker by the per-frame loop, and `sync_to_logic_tx` is fed by
    // the worker's onmessage handler.
    wasm_thread::Builder::new()
        .name("logic".to_owned())
        .spawn(move || {
            ambre_logic::run(
                ui_to_logic_rx,
                logic_to_ui_tx,
                logic_to_sync_tx,
                sync_to_logic_rx,
                logic_to_persist_tx,
            );
        })
        .expect("spawn Logic thread");

    // Dev-only multi-user testing: `?user=<name>` in the page URL authenticates
    // as that user (created on the fly server-side, keys derived deterministically
    // from the name). Lets several browser profiles collaborate on one machine
    // without OIDC. A prod server rejects the resulting `DevLogin` (DEV_MODE off).
    // The engine lives once now, so this fires only for the boot URL; the landing
    // dev-login field dispatches `DevLogin` directly thereafter (no reload).
    // `?user=` wins; otherwise fall back to the dev user remembered from a prior
    // session (localStorage) so a refresh of a `/trame/<id>` deep-link, which
    // carries no query string, still re-establishes the session. The dev keys are
    // re-derived deterministically from the name, so no key material is stored --
    // only the username. (Real-auth refresh-token persistence is Part D.)
    if let Some(username) = dev_user_from_url().or_else(stored_dev_user) {
        tracing::info!("Dev login requested for user '{username}'");
        remember_dev_user(&username);
        let _ = ui_to_logic_tx.try_send(LogicCommand::Rest {
            id: RequestId::next(),
            cmd: RestCommand::DevLogin { username },
        });
    }

    // Spawn the Sync worker: a SEPARATE wasm instance in its own Web Worker.
    // Its onmessage decodes `LogicEvent`s into `sync_to_logic_tx` (-> Logic).
    // Kept alive for the whole session by being captured in the relay closure.
    //
    // `sync_ready` gates the Logic->Sync relay: the worker installs its inbound
    // onmessage only at the end of its async wasm bootstrap, so any command
    // posted before then is dropped. The relay buffers commands (they wait in
    // `logic_to_sync_rx`) until the worker posts its readiness sentinel, which
    // flips this flag. Without it the boot `DevLogin` -- enqueued above, before
    // this spawn -- raced the bootstrap and was lost whenever the worker came up
    // slowly, leaving the session key unset and the document stuck hydrating.
    let sync_ready = Rc::new(Cell::new(false));
    let sync_worker = spawn_sync_worker(sync_to_logic_tx, Rc::clone(&sync_ready));

    // The canvas document-swap handle is created by `run_with` below (after the
    // chrome mounts the `<canvas>`); the route effect reads it through this slot.
    let switcher_slot: Rc<RefCell<Option<DocSwitcher>>> = Rc::new(RefCell::new(None));

    // ---- Mount the Leptos shell (once). Reactive state is created inside the
    // mount owner and pulled back out via slots for the imperative relay below. -
    let store_slot: Rc<RefCell<Option<CommandStates>>> = Rc::new(RefCell::new(None));
    let anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>> =
        Rc::new(RefCell::new(None));
    let toasts_slot: Rc<RefCell<Option<Toasts>>> = Rc::new(RefCell::new(None));
    let user_slot: Rc<RefCell<Option<RwSignal<Option<String>>>>> = Rc::new(RefCell::new(None));
    let docs_slot: Rc<RefCell<Option<RwSignal<Vec<DocItem>>>>> = Rc::new(RefCell::new(None));
    let members_slot: Rc<RefCell<Option<RwSignal<Vec<MemberInfo>>>>> = Rc::new(RefCell::new(None));
    let devmode_slot: Rc<RefCell<Option<RwSignal<Option<bool>>>>> = Rc::new(RefCell::new(None));
    let current_doc_slot: Rc<RefCell<Option<RwSignal<Option<DocumentId>>>>> =
        Rc::new(RefCell::new(None));
    let filter_popover_slot: Rc<
        RefCell<Option<RwSignal<Option<trame_chrome::FilterPopoverState>>>>,
    > = Rc::new(RefCell::new(None));
    let filter_values_slot: Rc<RefCell<Option<RwSignal<Vec<String>>>>> =
        Rc::new(RefCell::new(None));
    let find_state_slot: Rc<RefCell<Option<RwSignal<Option<trame_chrome::FindReplaceState>>>>> =
        Rc::new(RefCell::new(None));
    let find_result_slot: Rc<RefCell<Option<RwSignal<Option<(usize, usize)>>>>> =
        Rc::new(RefCell::new(None));
    let find_state_toile_slot: Rc<RefCell<Option<RwSignal<bool>>>> = Rc::new(RefCell::new(None));
    let find_result_toile_slot: Rc<
        RefCell<Option<RwSignal<Option<ambre_bus::app::toile::FindResults>>>>,
    > = Rc::new(RefCell::new(None));
    let style_slot: Rc<RefCell<Option<trame_chrome::StyleState>>> = Rc::new(RefCell::new(None));
    let chrome_status_slot: Rc<RefCell<Option<RwSignal<ChromeStatus>>>> =
        Rc::new(RefCell::new(None));

    mount_shell(
        ui_to_logic_tx.clone(),
        raf.clone(),
        theme_ctl.clone(),
        boot_doc,
        Rc::clone(&store_slot),
        Rc::clone(&anchor_slot),
        Rc::clone(&toasts_slot),
        Rc::clone(&user_slot),
        Rc::clone(&docs_slot),
        Rc::clone(&members_slot),
        Rc::clone(&devmode_slot),
        Rc::clone(&current_doc_slot),
        Rc::clone(&filter_popover_slot),
        Rc::clone(&filter_values_slot),
        Rc::clone(&find_state_slot),
        Rc::clone(&find_result_slot),
        Rc::clone(&find_state_toile_slot),
        Rc::clone(&find_result_toile_slot),
        boot_is_toile,
        Rc::clone(&style_slot),
        Rc::clone(&chrome_status_slot),
        formula_ref,
        formula_text,
        formula_action,
        sel_stats,
        sel_stats_toile,
        present_toile,
        presenter_index,
        presenter_count,
        presenter_notes,
        presenter_frame,
        broadcasting_toile,
        following_toile,
        presenter_peer_toile,
        perf_snap,
        zoom,
        ctx_scope,
        filter_active,
        sel_bounds,
        toile_ctx_scope,
        toile_menu_cmd,
        toile_active_tool,
        toile_style_model,
        toile_restyle,
        toile_float_bar,
        toile_quick_fill,
        sheets,
        active_sheet,
    );

    let store = store_slot
        .borrow()
        .clone()
        .expect("store created during mount");
    let anchor_pos = anchor_slot
        .borrow()
        .expect("anchor signal created during mount");
    let toasts = toasts_slot.borrow().expect("Toasts created during mount");
    let username = user_slot
        .borrow()
        .expect("user signal created during mount");
    let docs = docs_slot
        .borrow()
        .expect("docs signal created during mount");
    let members = members_slot
        .borrow()
        .expect("members signal created during mount");
    let dev_mode = devmode_slot
        .borrow()
        .expect("dev_mode signal created during mount");
    let current_doc = current_doc_slot
        .borrow()
        .expect("current_doc signal created during mount");
    // Shared filter popover signals (`-ui.md` sec.5.D), created in the mount owner
    // and read back here for the per-frame relay (which sets them on
    // `TrameEvent::ColumnValues` / `FilterApplied` / `FilterCleared`).
    let filter_popover = filter_popover_slot
        .borrow()
        .expect("filter_popover signal created during mount");
    let filter_values = filter_values_slot
        .borrow()
        .expect("filter_values signal created during mount");
    // Find/replace panel signals (`-ui.md` sec.5.G), read back for the keyboard
    // listener (Ctrl+F/Ctrl+H open the panel) and the per-frame relay (which sets
    // `find_result` on `TrameEvent::FindResult`).
    let find_state = find_state_slot
        .borrow()
        .expect("find_state signal created during mount");
    let find_result = find_result_slot
        .borrow()
        .expect("find_result signal created during mount");
    // Toile Find panel signals (`toile-ui §5quater`), read back for the keyboard
    // listener (Ctrl+F opens the panel) and the per-frame relay (which sets
    // `find_result_toile` on `ToileEvent::FindResults`).
    let find_state_toile = find_state_toile_slot
        .borrow()
        .expect("find_state_toile signal created during mount");
    let find_result_toile = find_result_toile_slot
        .borrow()
        .expect("find_result_toile signal created during mount");
    // Formatting-toolbar style state (`-ui.md` sec.5bis), read back for the
    // per-frame relay (which updates it on `TrameEvent::StyleApplied`).
    let style_state = style_slot
        .borrow()
        .expect("style state created during mount");
    // Chrome status indicator, read back for the per-frame relay (sets it on
    // `UiEvent::DocumentReady` / `ConnectionStatus`) and the route poll (arms
    // `hydrating` on a document switch).
    let chrome_status = chrome_status_slot
        .borrow()
        .expect("chrome_status signal created during mount");

    // Learn whether the server enables dev login (drives the signed-out panel).
    // Unauthenticated, Main-thread fetch -- a pre-session concern.
    wasm_bindgen_futures::spawn_local(async move {
        dev_mode.set(Some(fetch_dev_mode().await));
    });

    // Keyboard: resolve registered chords and dispatch to the routed document.
    install_command_keys(
        keymap,
        current_doc,
        ui_to_logic_tx.clone(),
        find_state,
        find_state_toile,
        toile_active_tool,
        boot_is_toile,
    );

    // Local persistence: open the IndexedDB store on this (Main) thread, where
    // the event loop is live. On open, restore the BOOT document's snapshot (if
    // any) so an edit survives a refresh without a network round-trip. The store
    // is shared with the per-frame relay below, which saves snapshots Logic emits.
    let persist_store: Rc<RefCell<Option<ambre_logic::persistence::PersistenceStore>>> =
        Rc::new(RefCell::new(None));
    {
        let store_slot = Rc::clone(&persist_store);
        let restore_tx = ui_to_logic_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let store = match ambre_logic::persistence::PersistenceStore::open()
                .recv_async()
                .await
            {
                Ok(Ok(store)) => store,
                Ok(Err(e)) => {
                    tracing::warn!("Persistence: store unavailable ({e})");
                    return;
                }
                Err(_) => {
                    tracing::warn!("Persistence: open channel dropped");
                    return;
                }
            };
            if let Some((doc_type, document_id)) = boot_doc_typed {
                if let Ok(Some(snapshot)) = store
                    .load_snapshot(*document_id.as_bytes())
                    .recv_async()
                    .await
                {
                    tracing::info!("Persistence: restoring doc ({} bytes)", snapshot.len());
                    let _ = restore_tx.try_send(LogicCommand::RestoreLocalSnapshot {
                        document_id,
                        doc_type,
                        snapshot,
                    });
                }
            }
            *store_slot.borrow_mut() = Some(store);
        });
    }

    // The single UI-thread dispatcher, pumped once per frame regardless of route:
    // persist drain, Logic->Sync relay, Logic->UI fan-out (command state to the
    // chrome, session/list to the landing, content to the view), anchor mirror.
    let anchor_pub_pump = anchor_pub.clone();
    let perf_pub_pump = perf_pub.clone();
    let filter_open_pump = filter_open_pub.clone();
    let persist_store_frame = Rc::clone(&persist_store);
    let mut last_anchor: Option<(f64, f64)> = None;
    let mut last_perf_push_ms: f64 = 0.0;
    let mut list_requested = false;
    let relay_tx = ui_to_logic_tx.clone();

    // SPA route binding, polled each frame on the main thread (where the `!Send`
    // canvas handle lives, so it stays OUT of the Leptos tree). The client URL is
    // the single source of truth: `leptos_router` pushState navigations and
    // browser back/forward both land here. On a change we close the previous
    // document, open the next, point the persistent canvas at it, gate the rAF
    // render (paint the doc / freeze it behind the landing overlay), and publish
    // the active document so every command trigger targets it.
    let route_switcher = Rc::clone(&switcher_slot);
    let route_tx = ui_to_logic_tx.clone();
    let route_raf = raf.clone();
    let mut prev_route: Option<route::Route> = None;
    let mut last_title: Option<String> = None;

    let pre_frame = move || {
        let cur_route = route::current();
        let cur_doc = cur_route.doc();
        if prev_route.as_ref() != Some(&cur_route) {
            let prev_doc = prev_route.as_ref().and_then(route::Route::doc);
            if prev_doc.map(|(_, id)| id) != cur_doc.map(|(_, id)| id) {
                if let Some((_, document_id)) = prev_doc {
                    let _ = route_tx.try_send(LogicCommand::CloseDocument { document_id });
                }
                if let Some((doc_type, document_id)) = cur_doc {
                    let _ = route_tx.try_send(LogicCommand::OpenDocument {
                        document_id,
                        doc_type,
                    });
                }
                current_doc.set(cur_doc.map(|(_, id)| id));
                // Arm the loading overlay when entering a document; clear it on
                // the landing route. The relay clears `hydrating` on the matching
                // `DocumentReady` (see the per-frame pump below).
                chrome_status.update(|s| {
                    s.hydrating = cur_doc.is_some();
                    if cur_doc.is_none() {
                        s.connected = false;
                    }
                });
            }
            match cur_doc {
                Some((_, document_id)) => {
                    if let Some(sw) = route_switcher.borrow().as_ref() {
                        sw.set_document(document_id);
                    }
                    route_raf.resume();
                }
                // Freeze the canvas behind the landing overlay (the relay below
                // keeps running -- only the render is gated).
                None => route_raf.pause(),
            }
            prev_route = Some(cur_route);
        }

        // Browser tab title. Icon-less context: compose the document's plaintext
        // name (decrypted into `docs`) with the generic localized type label
        // (`DocType::generic_label`, not the brand `name()`) -- `ambre-spec-chrome.md
        // 2.G`. Recomputed each frame so a rename or a late-arriving document list
        // updates the tab; the DOM write happens only when the title changes. An
        // open doc with no resolved name falls back to "Untitled - <label>"; the
        // landing is "Ambre".
        let title = match cur_doc {
            Some((doc_type, document_id)) => {
                let name = docs.with_untracked(|list| {
                    list.iter()
                        .find(|d| d.document_id == document_id)
                        .and_then(|d| d.name.clone())
                });
                let label = doc_type.generic_label();
                match name {
                    Some(n) if !n.trim().is_empty() => format!("{n} - {label}"),
                    _ => format!("Untitled - {label}"),
                }
            }
            None => "Ambre".to_owned(),
        };
        if last_title.as_deref() != Some(title.as_str()) {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                doc.set_title(&title);
            }
            last_title = Some(title);
        }

        // Persist Logic -> local store: drain snapshots and save them once the
        // store is open. Best-effort; the put() commits as the event loop idles.
        while let Ok((document_id, snapshot)) = logic_to_persist_rx.try_recv() {
            if let Some(store) = persist_store_frame.borrow().as_ref() {
                store.save_snapshot(*document_id.as_bytes(), snapshot);
            }
        }

        // Relay Logic -> Sync: drain commands and postMessage them to the Sync
        // worker (postcard, big payloads transferred zero-copy). `sync_worker` is
        // captured here, which is what keeps the worker alive for the session.
        // Held back until the worker signals readiness (`sync_ready`): commands
        // stay buffered in `logic_to_sync_rx` so none is posted into the worker's
        // bootstrap window where onmessage does not yet exist (order preserved).
        if sync_ready.get() {
            while let Ok(cmd) = logic_to_sync_rx.try_recv() {
                match postcard::to_allocvec(&cmd) {
                    Ok(bytes) => post_to_worker(&sync_worker, &bytes),
                    Err(e) => tracing::error!("[UI->Sync] failed to encode SyncCommand: {e}"),
                }
            }
        }

        while let Ok(ev) = logic_to_ui_rx.try_recv() {
            // Mirror authoritative command-toggle state into the chrome store.
            // The active app owns the event->toggle mapping; the launcher names
            // no content event variant.
            trame_chrome::mirror_event(&store, &ev);
            // Mirror the active cell's rich style into the formatting toolbar
            // (`-ui.md` sec.5bis). A no-op for non-style events.
            trame_chrome::apply_style_event(&style_state, &ev);
            // Surface Trame's user-facing notifications (rejections, validation
            // errors, sheet-lifecycle errors, sort/filter completion) as toasts.
            // The app owns the event->toast mapping; the launcher names no variant.
            trame_chrome::notify_event(&toasts, &ev);
            match &ev {
                // Non-fatal sync warnings surface as a transient warning toast.
                // They never reach the fatal path: the session keeps running and
                // the CRDT reconverges (a dropped/quarantined update is re-fetched
                // via RequestMissing once its parent snapshot lands).
                UiEvent::SyncWarning { code, .. } => toasts.warning(sync_warning_message(*code)),
                // Session bootstrap (dev `?user=` or OIDC): show the user, and
                // request the document list once for the landing overlay.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::SessionReady { display_name, .. },
                    ..
                } => {
                    username.set(Some(display_name.clone()));
                    if !list_requested {
                        list_requested = true;
                        let _ = relay_tx.try_send(LogicCommand::Rest {
                            id: RequestId::next(),
                            cmd: RestCommand::ListDocuments,
                        });
                    }
                }
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::DocumentList { documents },
                    ..
                } => {
                    docs.set(
                        documents
                            .iter()
                            .map(|d| DocItem {
                                document_id: d.document_id,
                                doc_type: d.doc_type,
                                name: d.name.clone(),
                                updated_at: d.updated_at,
                                member_count: d.member_count,
                            })
                            .collect(),
                    );
                }
                // A document was renamed or deleted (possibly from the launcher):
                // re-list so the card reflects the new name / disappears. Delete is
                // also removed optimistically by the card; re-listing reconciles.
                UiEvent::Rest {
                    evt:
                        ambre_bus::RestEvent::DocumentRenamed { .. }
                        | ambre_bus::RestEvent::DocumentDeleted { .. },
                    ..
                } => {
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocuments,
                    });
                }
                // Share dialog: the current member list landed -> render it.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::DocumentMembers { members: list, .. },
                    ..
                } => members.set(list.clone()),
                // An invite succeeded: confirm and refresh the member list.
                UiEvent::Rest {
                    evt:
                        ambre_bus::RestEvent::InviteSent {
                            document_id,
                            display_name,
                            ..
                        },
                    ..
                } => {
                    toasts.success(format!("Invited {display_name}"));
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocumentMembers {
                            document_id: *document_id,
                        },
                    });
                }
                // Revocation done (key rotation complete): confirm and refresh.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::RevocationComplete { document_id },
                    ..
                } => {
                    toasts.success("Member revoked");
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocumentMembers {
                            document_id: *document_id,
                        },
                    });
                }
                // A role change took effect: refresh the member list.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::MemberRoleUpdated { document_id, .. },
                    ..
                } => {
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocumentMembers {
                            document_id: *document_id,
                        },
                    });
                }
                // Chrome status indicator (`-chrome.md` 2.G): mirror the active
                // document's WebSocket link into the header label. Ignored for a
                // non-active document (a stale push during a fast doc switch).
                UiEvent::ConnectionStatus {
                    document_id,
                    connected,
                } if current_doc.get_untracked() == Some(*document_id) => {
                    chrome_status.update(|s| s.connected = *connected);
                }
                // Initial hydration finished: dismiss the loading overlay. Guarded
                // on the active document so a late ready for a doc we already left
                // does not clear the new doc's overlay.
                UiEvent::DocumentReady { document_id }
                    if current_doc.get_untracked() == Some(*document_id) =>
                {
                    chrome_status.update(|s| s.hydrating = false);
                }
                // A document was shared with us: refresh the landing list so it
                // appears without a manual reload, and notify the user.
                UiEvent::DocumentShared { .. } => {
                    toasts.info("A document was shared with you");
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocuments,
                    });
                }
                // Relayed service failures (e.g. unknown invitee) surface as a
                // warning toast so the share action gives feedback.
                UiEvent::ApiError { message, .. } => toasts.warning(message.clone()),
                // Shared filter popover (`-ui.md` sec.5.D): the distinct values of
                // the active column landed -> populate the "by values" list and pin
                // the popover to the real column index.
                UiEvent::Viewport {
                    event: AppEvent::Trame(ambre_bus::TrameEvent::ColumnValues { col, values }),
                    ..
                } => {
                    filter_values.set(values.clone());
                    filter_popover.update(|st| {
                        if let Some(st) = st {
                            st.col = *col;
                        }
                    });
                }
                // The filter took effect -> close the popover and flag the active
                // sheet as filtered (drives the context menu's dynamic entry).
                UiEvent::Viewport {
                    event: AppEvent::Trame(ambre_bus::TrameEvent::FilterApplied { .. }),
                    ..
                } => {
                    filter_popover.set(None);
                    filter_active.set(true);
                }
                // The filter was cleared -> close the popover and drop the flag.
                UiEvent::Viewport {
                    event: AppEvent::Trame(ambre_bus::TrameEvent::FilterCleared),
                    ..
                } => {
                    filter_popover.set(None);
                    filter_active.set(false);
                }
                // Sheet-tab strip (sec.13): the sheet list / active id changed ->
                // mirror it to the DOM `SheetTabs` signals. The canvas no longer
                // draws the tabs, so this is the only consumer now.
                UiEvent::Viewport {
                    event:
                        AppEvent::Trame(ambre_bus::TrameEvent::SheetList {
                            sheets: list,
                            active,
                        }),
                    ..
                } => {
                    sheets.set(list.clone());
                    active_sheet.set(active.clone());
                }
                // Status bar (sec.13bis): selection aggregates for a multi-cell
                // selection. The view clears the signal on a single cell; here we
                // fill it from the Logic-computed totals.
                UiEvent::Viewport {
                    event:
                        AppEvent::Trame(ambre_bus::TrameEvent::SelectionStats {
                            sum,
                            avg,
                            count,
                            count_num,
                            min,
                            max,
                        }),
                    ..
                } => sel_stats.set(Some(trame_chrome::SelStats {
                    sum: *sum,
                    avg: *avg,
                    count: *count,
                    count_num: *count_num,
                    min: *min,
                    max: *max,
                })),
                // Find & Replace (sec.5.G): summarize the match set for the panel's
                // "k/n" counter. The view (canvas) consumes the same event to move
                // the active cell onto the focused match and highlight the set.
                UiEvent::Viewport {
                    event: AppEvent::Trame(ambre_bus::TrameEvent::FindResult { matches, active }),
                    ..
                } => {
                    let total = matches.len();
                    find_result.set(match active {
                        Some(i) if total > 0 => Some((*i + 1, total)),
                        _ => Some((0, total)),
                    });
                }
                // Toile Find (§5quater): feed the panel's counter and step origin.
                // The canvas view consumes the same event (forwarded below) to
                // highlight the hits and recenter the camera on `focus`.
                UiEvent::Viewport {
                    event: AppEvent::Toile(ambre_bus::app::toile::ToileEvent::FindResults(fr)),
                    ..
                } => find_result_toile.set(Some(fr.clone())),
                // Toile status bar (§8bis): selection aggregates for the band.
                // The canvas view requests them on each selection change and also
                // receives this event (forwarded below), consuming it only for a
                // pending Fit / find-focus recenter; here we mirror count/bounds
                // into the status-bar signal.
                UiEvent::Viewport {
                    event: AppEvent::Toile(ambre_bus::app::toile::ToileEvent::SelectionStats(s)),
                    ..
                } => {
                    sel_stats_toile.set(Some(toile_chrome::SelStats::from_reply(s.count, s.bounds)))
                }
                // Presenter overlay panel (§1.D): show / hide it with focus mode,
                // and mirror the enriched slide payload into the panel signals.
                // The canvas view consumes the same events (forwarded below) for
                // the camera glide + Smart Animate pass.
                UiEvent::Viewport {
                    event:
                        AppEvent::Toile(ambre_bus::app::toile::ToileEvent::PresentModeChanged(on)),
                    ..
                } => present_toile.set(*on),
                UiEvent::Viewport {
                    event: AppEvent::Toile(ambre_bus::app::toile::ToileEvent::SlideChanged(c)),
                    ..
                } => {
                    presenter_index.set(c.index);
                    presenter_count.set(c.count);
                    presenter_notes.set(c.notes.clone());
                    presenter_frame.set(c.frame.clone());
                }
                // "Follow presenter" role changes (§1.B): reflect the local
                // broadcast toggle and follow target in the panel signals.
                UiEvent::Viewport {
                    event:
                        AppEvent::Toile(ambre_bus::app::toile::ToileEvent::PresentBroadcastChanged(on)),
                    ..
                } => broadcasting_toile.set(*on),
                UiEvent::Viewport {
                    event: AppEvent::Toile(ambre_bus::app::toile::ToileEvent::FollowingChanged(who)),
                    ..
                } => following_toile.set(*who),
                // A peer broadcasting its playback position (§1.B): remember it as
                // the current presenter so the Follow affordance appears. The event
                // still flows to the view below for the overlay pass.
                UiEvent::Viewport {
                    event: AppEvent::Toile(ambre_bus::app::toile::ToileEvent::PeerPresence(u)),
                    ..
                } => {
                    if u.presence.present_slide.is_some() {
                        presenter_peer_toile.set(Some(u.user_id));
                    }
                }
                _ => {}
            }
            // Content (and everything else) still flows to the view unchanged.
            let _ = view_tx.try_send(ev);
        }

        // Mirror the view's published selection anchor into the chrome signal,
        // only on change so the floating toolbar repositions just when it moves.
        let next = anchor_pub_pump.read().map(|a| (a.x, a.y));
        if next != last_anchor {
            last_anchor = next;
            anchor_pos.set(next);
        }

        // Open the shared filter popover when the canvas raised the one-shot
        // intent (`-ui.md` sec.5.D): a click on an in-canvas autofilter button.
        // The view already moved the active cell onto the button's column (so the
        // anchor mirrored just above points at it) and asked nothing else; this
        // runs the exact body of the context-menu `FILTER_COLUMN` branch, so the
        // two entry points share one filter code path.
        if filter_open_pump.take() {
            let (x, y) = client_anchor(anchor_pos.get_untracked());
            // `col` is provisional; the ColumnValues reply carries the real one.
            filter_popover.set(Some(trame_chrome::FilterPopoverState { col: 0, x, y }));
            filter_values.set(Vec::new());
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = relay_tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::RequestActiveColumnValues),
                });
            }
        }

        // Mirror the view's perf snapshot into the status-bar signal, throttled
        // to ~2 Hz: the old wgpu HUD refreshed every 500 ms too, and a per-frame
        // DOM update would be wasteful for a debug read-out.
        let now_ms = web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now());
        if now_ms - last_perf_push_ms >= 500.0 {
            last_perf_push_ms = now_ms;
            perf_snap.set(perf_pub_pump.read());
        }
    };

    // Boot the canvas (once). It grabs the `<canvas>` the chrome just mounted,
    // drives the Trame view through the rAF loop, and returns the document-swap
    // handle the route effect uses on navigation.
    let view_tx_logic = ui_to_logic_tx.clone();
    let formula_ref_tv = formula_ref;
    let formula_text_tv = formula_text;
    let formula_action_tv = formula_action;
    let switcher = host::run_with(
        "ambre-canvas",
        move |w, h, text_input| {
            // Pick the view by the booted document's type. Cross-type in-session
            // navigation is out of Phase 0: the `DocSwitcher` only swaps the id
            // within one `AppView`, so a Trame<->Toile hop reloads via the route.
            if boot_is_toile {
                AppView::Toile(ToileView::new(
                    initial_doc,
                    view_tx_logic,
                    view_rx,
                    w,
                    h,
                    text_input,
                    perf_pub,
                    theme_ctl,
                    toile_ctx_scope,
                    toile_menu_cmd,
                    toile_active_tool,
                    zoom,
                    sel_stats_toile,
                    toile_style_model,
                    toile_restyle,
                    toile_float_bar,
                    toile_quick_fill,
                ))
            } else {
                AppView::Trame(TrameView::new(
                    initial_doc,
                    view_tx_logic,
                    view_rx,
                    w,
                    h,
                    text_input,
                    anchor_pub,
                    perf_pub,
                    filter_open_pub,
                    theme_ctl,
                    formula_ref_tv,
                    formula_text_tv,
                    formula_action_tv,
                    sel_stats,
                    ctx_scope,
                    sel_bounds,
                    zoom,
                ))
            }
        },
        pre_frame,
        raf,
    );
    // Publish the canvas swap handle for the per-frame route poll. The poll
    // reruns every frame, so it binds the routed document as soon as this lands.
    *switcher_slot.borrow_mut() = Some(switcher);
}

// The launcher is split across sibling modules; `shell`/`platform` inherit the
// launcher's imports (and `Wiring`/`ChromeStatus`) via `use super::*`.
mod platform;
mod shell;

use platform::*;
use shell::mount_shell;
