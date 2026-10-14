// ambre-app/src/lib.rs
// ============================================================================
// Launcher -- the cdylib entry point (#[wasm_bindgen(start)]), running on the
// page's main JS thread (the UI thread).
//
// SPA model (ambre-spec-architecture): the ENGINE is created once and lives for
// the whole session. That is the inter-thread queues, the Logic thread, the
// Sync worker, the local-persistence store, and the wgpu canvas. A
// `leptos_router` shell drives a CLIENT-side route: pushState, no full page
// load. Navigating between the document list and a document only swaps the
// canvas's active document in place (`DocSwitcher`). The Sync worker keeps its
// in-memory session keys across a navigation.
//
//   - UI (this main thread) + Logic (a `wasm_thread` worker) share this Main
//     wasm instance's memory and talk over `ambre_bus::conduit` queues.
//   - Sync is a SEPARATE wasm instance in its own Web Worker (own memory),
//     spawned here via `web_sys::Worker` and reached over `postMessage`. The
//     main thread relays Logic<->Sync traffic in the single per-frame relay.
//   - The Leptos shell mounts once: the AppShell chrome (which owns the
//     `<canvas>`) plus a landing overlay shown on the `/` route.
//   - One route-independent rAF loop (the canvas host's) runs the per-frame
//     relay AND the dispatcher, whatever the route. It is paused, on a frozen
//     frame, while the landing overlay covers the canvas.
//
// Command model (ambre-spec-commands.md): every trigger -- toolbar button,
// keyboard shortcut, context menu -- produces the SAME `CommandId` ->
// `LogicCommand`, targeting the currently routed document (`current_doc`).
//
// Compiles ONLY on wasm32 targets (depends on web_sys, wgpu, leptos).
// ============================================================================

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use ambre_bus::conduit;
use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{
    AppCommand, AppEvent, DocumentId, LogicCommand, LogicEvent, MemberInfo, RequestId, RestCommand,
    Role, SyncCommand, UiEvent, UserId,
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
/// The local store the Main thread drives, within its budget (`RESID-002`).
mod local_store;
mod route;
mod signin;
/// The recipient's key, compared and confirmed before a share.
mod tofu;

// The crate's unit tests (pure helpers in `route`/`host`) run under headless
// Chrome via `wasm-pack test --headless --chrome` (`just test-wasm`). `--chrome`
// sets `WASM_BINDGEN_TEST_ONLY_WEB`, so the runner skips any test binary not
// configured for the browser -- declare it once here for the whole lib-test.
#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

use host::{AppView, ClipboardPaste, DocSwitcher};

use landing::{DocItem, LandingSignals};
use tofu::TofuPrompt;

/// Capacity of every inter-thread queue.
///
/// Deliberately generous: P3 traffic is request/response and never approaches
/// it. The hot viewport path is coalesced to <=1 send/frame on the UI side
/// (see `TrameView`). It exists purely as a safety bound so a momentarily-slow
/// consumer cannot grow an unbounded queue in the shared heap.
const CHANNEL_CAPACITY: usize = 1024;

/// Sync events held past a full channel. At the high count the worker stops
/// reading the relay, and at the low count it starts again
/// (`ambre-spec-budgets.md section 6`, `SYNC-059`).
const SYNC_INBOX_HIGH: usize = 4 * CHANNEL_CAPACITY;
const SYNC_INBOX_LOW: usize = CHANNEL_CAPACITY;

/// The child fan-out registry: every live view of a nested document, by
/// document. A list rather than a slot -- two embeds of one document are two
/// views (`TOILE-UI-097`).
type ChildRoutes = Rc<RefCell<HashMap<DocumentId, Vec<ambre_bus::conduit::Sender<UiEvent>>>>>;

/// What a child's queue could not take yet, oldest first, by document
/// (`TOILE-UI-340`).
type ChildBacklog = Rc<RefCell<HashMap<DocumentId, VecDeque<UiEvent>>>>;

/// How many events may wait for one child before the oldest are dropped.
///
/// A child is pumped once per frame, so a backlog only forms on a burst larger
/// than the queue itself. Past this the client is saturated for good and no
/// amount of memory fixes it. The real answer is the busy signal `BUS-016`
/// prescribes, which lands with its own Story. Until then the bound is what
/// keeps a stalled child from growing without limit, and it says so out loud.
const CHILD_BACKLOG_LIMIT: usize = CHANNEL_CAPACITY;

/// Hand `ev` to every live view of `id`, deferring what a full queue refuses
/// (`TOILE-UI-340`).
///
/// The old path dropped it: a `Full` fell through the disconnected test and then
/// past the `continue`. The event reached neither the child nor the board. It is
/// a CRDT delta, so the child's view then sat behind its own document until
/// something else redrew it from scratch.
///
/// A document with anything already waiting takes the new event straight to the
/// back of its queue. A child must not see a later delta before an earlier one.
fn send_to_child(routes: &ChildRoutes, backlog: &ChildBacklog, id: DocumentId, ev: UiEvent) {
    if backlog.borrow().get(&id).is_some_and(|q| !q.is_empty()) {
        defer_for_child(backlog, id, ev);
        return;
    }
    if !offer_to_child(routes, id, &ev) {
        defer_for_child(backlog, id, ev);
    }
}

/// Offer `ev` to each live view of `id`, dropping the senders whose view is
/// gone. Reports whether every remaining one took it; `false` means at least one
/// queue was full and the event must be re-offered.
fn offer_to_child(routes: &ChildRoutes, id: DocumentId, ev: &UiEvent) -> bool {
    let mut routes = routes.borrow_mut();
    let Some(senders) = routes.get_mut(&id) else {
        // No view left at all: the document is no longer shown, so the event has
        // nowhere to go and nothing to be behind.
        return true;
    };
    let mut all_took_it = true;
    senders.retain(|tx| match tx.try_send(ev.clone()) {
        Ok(()) => true,
        // The child view was dropped; reclaim its slot.
        Err(ambre_bus::conduit::TrySendError::Disconnected(_)) => false,
        Err(ambre_bus::conduit::TrySendError::Full(_)) => {
            all_took_it = false;
            true
        }
    });
    // The entry stays even with no sender left in it, and that emptiness is what
    // holds `TOILE-UI-098`. A child view is dropped the frame its embed leaves
    // the culled set, while its session stays open for the whole close debounce.
    // An event arriving in that window used to find no route and fall through to
    // the board's own stream and chrome mirrors. A Toile nested in a Toile emits
    // exactly the event the parent board would then apply to itself. An empty
    // entry absorbs it instead, and the next view of that document pushes its
    // sender back into the same slot.
    //
    // What is left is a few dozen bytes per document embedded at least once in
    // this session, which nothing reclaims. Bounded by the boards a user opens,
    // and the trade the other way is a delta applied to the wrong document.
    all_took_it
}

/// Queue `ev` for `id`, oldest first, and say so when the bound forces a drop.
fn defer_for_child(backlog: &ChildBacklog, id: DocumentId, ev: UiEvent) {
    let mut backlog = backlog.borrow_mut();
    let queue = backlog.entry(id).or_default();
    queue.push_back(ev);
    if queue.len() > CHILD_BACKLOG_LIMIT {
        queue.pop_front();
        tracing::error!("embedded document {id}: backlog full, an event was dropped");
    }
}

/// Re-offer every deferred event, oldest first, stopping at the first one a
/// queue still refuses so the order is kept (`TOILE-UI-340`).
fn flush_child_backlog(routes: &ChildRoutes, backlog: &ChildBacklog) {
    let ids: Vec<DocumentId> = backlog.borrow().keys().copied().collect();
    for id in ids {
        loop {
            let Some(ev) = backlog.borrow().get(&id).and_then(|q| q.front().cloned()) else {
                break;
            };
            if !offer_to_child(routes, id, &ev) {
                break;
            }
            backlog.borrow_mut().get_mut(&id).map(VecDeque::pop_front);
        }
        if backlog.borrow().get(&id).is_some_and(VecDeque::is_empty) {
            backlog.borrow_mut().remove(&id);
        }
    }
}

#[derive(Clone)]
struct Wiring {
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    theme_ctl: ambre_canvas::ThemeControl,
    /// The active app's command toggle signals (the store's reactive view,
    /// chrome side). Opaque to the launcher -- the app's chrome reads its own.
    toggles: trame_chrome::Toggles,
    /// Toile's Undo/Redo availability signals, resolved from its own store so the
    /// menu bar disables the two rows from the Logic thread's `UndoState` alone
    /// (`UNDO-006`).
    toile_toggles: toile_chrome::Toggles,
    /// The active cell's rich formatting, driving the formatting toolbar's
    /// font/size selects, colour pickers, alignment, number format, borders, wrap
    /// and underline controls (`trame-cell-styles`). Fed by `TrameEvent::StyleApplied`.
    style_state: trame_chrome::StyleState,
    /// Transient notifications (ARIA live region).
    toasts: Toasts,
    /// Signed-in user's display name; `None` = signed out.
    username: RwSignal<Option<String>>,
    /// The user's documents (landing list).
    docs: RwSignal<Vec<DocItem>>,
    /// Members of the active document (share dialog). Populated by the per-frame
    /// relay on `DocumentMembers`; empty until the dialog requests the list.
    members: RwSignal<Vec<MemberInfo>>,
    /// Pending TOFU trust prompt (`ambre-spec-key-verification.md`), or `None` when no
    /// invitation is awaiting the user's fingerprint decision. Set by the relay on
    /// a New/Changed key; the confirmation dialog in `AppRoot` reads it.
    tofu_prompt: RwSignal<Option<TofuPrompt>>,
    /// Set by `AppRoot` when the user confirms a TOFU prompt. The relay polls it,
    /// pins the key in `trusted_keys`, and dispatches phase 2 of the invite.
    tofu_confirm: RwSignal<Option<TofuPrompt>>,
    /// Role selected in the invite form, carried across the two-phase invite. The
    /// phase-1 `FetchInviteeKey` does not carry it, so both the confirm path and
    /// the auto-proceed (Trusted) path grant the intended role.
    invite_role: RwSignal<Role>,
    /// The signed-in user, set on `SessionReady`. The share dialog reads it to
    /// tell the user's own row from the others (`SYNC-141`).
    me: RwSignal<Option<UserId>>,
    /// Whether the server enables dev login; `None` until `/api/auth/config` answers.
    dev_mode: RwSignal<Option<bool>>,
    /// The currently routed document, or `None` on the landing route. Read by
    /// every command trigger so it targets the active document.
    current_doc: RwSignal<Option<DocumentId>>,
    /// Formula bar: active cell reference label (for example "B2").
    formula_ref: RwSignal<String>,
    /// Formula bar: cell raw text / live edit buffer.
    formula_text: RwSignal<String>,
    /// Formula bar: commit/cancel action from the HTML input.
    formula_action: RwSignal<Option<FormulaAction>>,
    /// Shared filter popover open state (`trame-shared-filter`); `None` = closed.
    filter_popover: RwSignal<Option<trame_chrome::FilterPopoverState>>,
    /// The active column's distinct visible values, feeding the popover's "by
    /// values" list. Set by the per-frame relay on `TrameEvent::ColumnValues`.
    filter_values: RwSignal<Vec<String>>,
    /// Find/replace panel open state (`trame-find-replace`); `None` = closed.
    find_state: RwSignal<Option<trame_chrome::FindReplaceState>>,
    /// Last `FindResult` summarized as `(active_1based, total)` for the panel's
    /// "k/n" counter. Set by the per-frame relay on `TrameEvent::FindResult`.
    find_result: RwSignal<Option<(usize, usize)>>,
    /// Whether the booted document is a Toile board. The chrome layer is still
    /// largely Trame-shaped; this gates the Toile-specific panels (the Find panel,
    /// `toile-find`) which replace their Trame equivalents. Cross-type
    /// in-session navigation reloads the page, so it is fixed per page load.
    boot_is_toile: bool,
    /// Whether the booted document is a Prose document: the canvas mounts the
    /// page view and the status bar shows the five figures.
    boot_is_prose: bool,
    /// The five figures of the open Prose document, for the status bar slot
    /// (`PROSE-FEAT-040`). Published by the Prose view.
    prose_figures: RwSignal<Option<[String; 5]>>,
    /// The comment panel and the suggestion popover of the open Prose
    /// document (`PROSE-UI-050`, `PROSE-UI-052`). Published by the Prose view.
    prose_signals: host::ProseSignals,
    /// Toile Find panel open flag (`toile-find`); Ctrl+F sets it.
    find_state_toile: RwSignal<bool>,
    /// Last Toile `FindResults` (hits + focus), feeding the panel's counter and
    /// step origin. Set by the per-frame relay on `ToileEvent::FindResults`.
    find_result_toile: RwSignal<Option<ambre_bus::app::toile::FindResults>>,
    /// Presenter overlay panel open flag (`toile-presentation`). Driven by the
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
    /// "Follow presenter" role signals (`toile-presentation`). `broadcasting`
    /// mirrors the local broadcast toggle, `following` the active follow target,
    /// and `presenter_peer` the latest peer seen broadcasting (drives the panel's
    /// Follow affordance).
    broadcasting_toile: RwSignal<bool>,
    following_toile: RwSignal<Option<ambre_bus::UserId>>,
    presenter_peer_toile: RwSignal<Option<ambre_bus::UserId>>,
    /// Toile right-click context-menu scope (`toile-menus`): the canvas view
    /// publishes it on each right-click. The shell's entry `Memo` reads it to build
    /// the menu, omitting inapplicable rows.
    toile_ctx_scope: RwSignal<ambre_toile::ui::menu::CtxScope>,
    /// Toile active tool (`toile-tools`): the shared source of truth for the DOM
    /// tool palette and the bare-key tool chords. The canvas view reads it each
    /// frame and applies it. Tool selection is UI-only, canvas-local state.
    toile_active_tool: RwSignal<ambre_toile::ui::Tool>,
    /// Toile canvas cursor (`toile-tools`). The view publishes the
    /// position-dependent CSS cursor: directional resize on a selection edge /
    /// corner, grab on the rotate handle, `grabbing` while moving. `None` falls
    /// back to the tool's base `cursor_css`. Read by the chrome's cursor `Effect`.
    toile_cursor: RwSignal<Option<std::borrow::Cow<'static, str>>>,
    /// Toile style-inspector model (`toile-styles`): the selection's common
    /// `ElementStyle` + mixed-field mask, published by the canvas view. The DOM
    /// inspector reads it to render values and mixed-state. `None` hides it.
    toile_style_model: RwSignal<Option<toile_chrome::StyleModel>>,
    /// Whole-style edit the inspector emits back (`toile-styles`): the
    /// canvas view drains it into one `RestyleElements` over the live selection.
    toile_restyle: RwSignal<Option<ambre_bus::app::toile::ElementStyle>>,
    /// Floating selection toolbar placement + flags (`toile-styles`),
    /// republished every frame by the canvas view so the bar tracks pan/zoom;
    /// `None` hides it.
    toile_float_bar: RwSignal<Option<toile_chrome::FloatBar>>,
    /// Single-field style edit the floating bar emits back (`toile-styles`).
    /// The canvas view applies it on top of the live common style as one
    /// `RestyleElements` over the selection.
    toile_quick_style: RwSignal<Option<toile_chrome::StyleEdit>>,
    /// The comments panel's signals (`toile-comments`): the canvas view writes
    /// the list, the open thread and the drop composer. It drains the intents
    /// the panel and the composer emit. The panel holds nothing of its own
    /// (`TOILE-UI-023`).
    toile_comments: toile_chrome::CommentSignals,
    /// The deck panel's signals (`toile-presentation`): the canvas view writes
    /// the board's frames and drains the panel's intents. The panel holds no
    /// order of its own.
    toile_deck: toile_chrome::DeckSignals,
    /// A file the shell read for the board (`toile-import-export`): a drop on
    /// the canvas or a pick from the File menu. The canvas view drains it and
    /// lands it at the drop point (`TOILE-IO-023`).
    toile_dropped_file: RwSignal<Option<toile_chrome::DroppedFile>>,
    /// The embed the shell's address dialog / document picker resolved, awaiting
    /// the view that will land it on the board (`toile-embeds`).
    toile_placed_embed: RwSignal<Option<toile_chrome::PlacedEmbed>>,
    /// A link edit the shell's chooser produced, drained by the canvas view and
    /// applied to its single selection as one `SetLink` (`toile-links`).
    toile_set_link: RwSignal<Option<toile_chrome::LinkEdit>>,
    /// A followed link the board handed back because only the shell can perform
    /// it: a sandboxed tab, or another document (`TOILE-065`, `TOILE-114`).
    toile_followed_link: RwSignal<Option<toile_chrome::ShellNav>>,
    /// Status-bar selection aggregates (`trame-status-bar`); `None` on a single
    /// cell. Set by the per-frame relay on `TrameEvent::SelectionStats`, cleared
    /// by the canvas view on a single-cell selection.
    sel_stats: RwSignal<Option<trame_chrome::SelStats>>,
    /// Toile status-bar selection aggregates (toile-status-bar): element tally +
    /// combined world bounds; `None` on an empty selection. Set by the per-frame
    /// relay on `ToileEvent::SelectionStats`.
    sel_stats_toile: RwSignal<Option<toile_chrome::SelStats>>,
    /// Status-bar perf snapshot (fps + frame timings), pumped from the canvas
    /// view's `PerfPublisher` by the per-frame relay (~2 Hz).
    perf_snap: RwSignal<ambre_canvas::PerfSnapshot>,
    /// Grid zoom factor (`1.0` = 100%, `trame-status-bar`): the status-bar menu
    /// writes it, the canvas view polls it each frame. Local view state, not synced.
    zoom: RwSignal<f64>,
    /// Right-click context-menu scope (`trame-context-menu`): the view sets it
    /// (cell vs header) before the DOM menu opens; the shell derives the layout.
    ctx_scope: RwSignal<trame_chrome::CtxScope>,
    /// Whether the active sheet currently has a shared filter (`trame-shared-filter`):
    /// mirrored from `TrameEvent::FilterApplied`/`FilterCleared`. Drives the
    /// context menu's dynamic filter entry (Filter by column XOR Remove filter).
    filter_active: RwSignal<bool>,
    /// Multi-column sort dialog open state (`trame-sort`); `None` = closed.
    sort_dialog: RwSignal<Option<trame_chrome::SortDialogState>>,
    /// Sheet-tab strip model (`trame-sheets`): `(id, name)` in display order.
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
    /// The command path: the booted application's handler, and what the shell
    /// performs of its answers (`ARCH-018`).
    commands: shell::CommandPath,
    /// Toile's three choosers: a web embed, a document embed, a link.
    toile_dialogs: toile_chrome::ToileDialogs,
    /// Trame's panels' state, and the commands their intents become.
    trame_relay: trame_chrome::TrameRelay,
    /// Toile's panels' state, and the commands their intents become.
    toile_relay: toile_chrome::ToileRelay,
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
// entry point, which collides with this `#[wasm_bindgen(start)] fn main` ("entry
// symbol `main` declared multiple times"). The launcher boot path is not what the
// unit tests exercise (they cover the pure helpers -- route/base64/fps). Dropping
// `main` from the test build is harmless.
/// Boot the client: the Logic thread, the Sync worker and the shell.
///
/// # Panics
///
/// When the browser refuses a thread or the page lacks its mount points.
#[cfg(not(test))]
#[expect(
    clippy::expect_used,
    reason = "the boot spawns the threads the page runs on, which has no fallback page"
)]
#[wasm_bindgen(start)]
pub fn main() {
    // Our crates log at the build's level, and every dependency at WARN
    // alone. Loro logs its encoding at INFO on each snapshot. The wasm layer
    // turns every event into a console line and a timing mark. On the Logic
    // thread, that was most of a keystroke's pass
    // (`ambre-spec-budgets.md` section 1). Timing marks are a debug aid, so a
    // release build does not stamp them.
    let level = if cfg!(debug_assertions) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let filter = tracing_subscriber::filter::Targets::new()
        .with_target("ambre", level)
        .with_default(tracing::Level::WARN);
    let layer = tracing_wasm::WASMLayer::new(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(level)
            .set_report_logs_in_timings(cfg!(debug_assertions))
            .build(),
    );
    use tracing_subscriber::layer::SubscriberExt as _;
    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(layer).with(filter),
    );
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        let msg = info.to_string();
        // Latch the canvas fatal screen. Works from any thread sharing this
        // instance's memory (UI/Logic). A Logic-thread panic has no `window`,
        // so the JS overlay below cannot fire. The UI thread's rAF loop is
        // still alive, and will paint the terminal screen from this latch.
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

    // Inter-thread queues (`ambre_bus::conduit`, in this Main instance's shared heap).
    //   UI    --LogicCommand--> Logic      Logic --UiEvent--> UI
    //   Logic --SyncCommand---> [main relay] --postMessage--> Sync worker
    //   Sync worker --postMessage--> [main relay] --LogicEvent--> Logic
    //
    // Bounded (backpressure), and lock-free (`ARCH-020`). The UI runs on the page
    // main thread, where the browser refuses `Atomics.wait`. So every end it holds
    // only tries (`ARCH-021`): a lossy send drops on `Full`, a lossless one is held.
    // Logic parks on `logic_bell`, which every channel into it rings (`ARCH-022`),
    // and parks on a full outbound channel until the relay drains it (`ARCH-023`).
    // The Logic<->Sync halves on the main side (`logic_to_sync_rx`,
    // `sync_to_logic_tx`) are owned by the per-frame relay below, NOT by a worker.
    let logic_bell = conduit::Doorbell::new();
    let (ui_to_logic_tx, ui_to_logic_rx) =
        conduit::to_worker::<LogicCommand>(CHANNEL_CAPACITY, &logic_bell);
    let (logic_to_ui_tx, logic_to_ui_rx) =
        conduit::to_main::<UiEvent>(CHANNEL_CAPACITY, &logic_bell);
    let (logic_to_sync_tx, logic_to_sync_rx) =
        conduit::to_main::<SyncCommand>(CHANNEL_CAPACITY, &logic_bell);
    let (sync_to_logic_tx, sync_to_logic_rx) =
        conduit::to_worker::<LogicEvent>(CHANNEL_CAPACITY, &logic_bell);
    // What the Sync -> Logic channel could not take yet, re-offered each frame.
    let sync_inbox: SyncInbox = Rc::new(RefCell::new(conduit::Backlog::with_marks(
        sync_to_logic_tx,
        SYNC_INBOX_HIGH,
        SYNC_INBOX_LOW,
    )));
    // Logic -> Main, carrying document snapshots and deltas for local
    // persistence. The Main thread owns the IndexedDB store (its callbacks need a
    // live event loop, which the blocking Logic loop cannot offer). It drains
    // this in `pre_frame`.
    let (logic_to_persist_tx, logic_to_persist_rx) =
        conduit::to_main::<(DocumentId, ambre_logic::PersistWrite)>(CHANNEL_CAPACITY, &logic_bell);

    // Dispatcher -> canvas view: the content event stream, forwarded after the
    // dispatcher has mirrored command-state events into the chrome.
    let (view_tx, view_rx) = conduit::local::<UiEvent>(CHANNEL_CAPACITY);

    // Per-document Logic->UI fan-out for nested `doc_embed` children. A child
    // view built by `make_child` (below) registers a demultiplexed sender here.
    // The relay routes every doc-tagged event whose `document_id` is a live child
    // to those queues, instead of the shared top-level `view_tx`. So a child's
    // stream never reaches the parent view (which ignores `document_id`).
    // Dropping a child drops its receiver, so the relay reclaims that slot on its
    // next send to it.
    //
    // A LIST of senders per document, not one. Two embeds of a single document
    // are two views (`TOILE-UI-097`), each with its own queue, and both must
    // see the document's events. One slot per document had the second
    // registration evict the first, leaving that embed painting a view nothing
    // fed.
    let child_routes: ChildRoutes = Rc::new(RefCell::new(HashMap::new()));
    // What a child's queue could not take yet (`TOILE-UI-340`).
    let child_backlog: ChildBacklog = Rc::new(RefCell::new(HashMap::new()));

    // Command keymap (the keyboard view of the registry). The active app binds
    // its own chords -- the launcher names none. Populated once the boot document
    // type is resolved below (Toile binds a different, smaller set than Trame).
    let mut keymap = Keymap::new();

    // Handle to the canvas rAF loop, so a fullscreen modal -- or the landing
    // overlay covering the canvas -- can freeze it.
    let raf = ambre_canvas::RafControl::new();

    // Canvas->DOM selection anchor: the view publishes the active cell's screen
    // position, and the dispatcher mirrors it into the chrome's `anchor_pos`
    // signal. The floating selection toolbar transform-syncs onto it.
    let anchor_pub = ambre_canvas::AnchorPublisher::new();

    // Canvas->DOM perf signal: the view publishes the per-frame perf snapshot
    // (fps + timings), the dispatcher mirrors it into the `perf_snap` signal the
    // status bar renders. The HUD left the wgpu overlay (`trame-status-bar`)
    // -- no point shaping debug glyphs on the GPU every frame.
    let perf_pub = ambre_canvas::PerfPublisher::new();

    // Canvas->DOM filter-open signal (`trame-shared-filter`): the view raises this
    // one-shot intent when an in-canvas autofilter button is clicked. The per-frame
    // relay consumes it (after mirroring the anchor) and opens the DOM
    // FilterPopover through the same path as the context-menu "Filter by column"
    // entry. So there is one filter code path with two entry points.
    let filter_open_pub = ambre_canvas::FilterOpenPublisher::new();

    // Canvas->DOM busy signal (`BUS-016`): the active view raises this level
    // while its lossless outbox holds a command the Logic thread has not taken.
    // The view refuses input for as long as it is up, so the shell owes the user
    // a reason. One publisher covers the session: an embedded child sends on the
    // same signal as its host board, so both meet the same saturation.
    let busy_pub = ambre_canvas::BusyPublisher::new();
    // Canvas->DOM notices: what a board view must tell the user and cannot show
    // on the board, such as an edit lost with its element. An embedded child
    // queues on the same publisher as its host board.
    let notice_pub = ambre_canvas::NoticePublisher::new();

    // Formula bar signals (HTML input -> TrameView sync). RwSignal is Copy, so
    // we create them once and pass clones to both TrameView and the Leptos shell.
    let formula_ref = RwSignal::new(String::new());
    let formula_text = RwSignal::new(String::new());
    let formula_action = RwSignal::new(None::<FormulaAction>);

    // TOFU share-dialog signals (`ambre-spec-key-verification.md`), created here in the
    // launcher owner and shared with both `AppRoot` (dialog) and the per-frame
    // relay (trust decision + phase-2 dispatch).
    let tofu_prompt = RwSignal::new(None::<TofuPrompt>);
    let tofu_confirm = RwSignal::new(None::<TofuPrompt>);
    let invite_role = RwSignal::new(Role::Editor);
    let me = RwSignal::new(None::<UserId>);

    // Status-bar signals (`trame-status-bar`): selection aggregates, set from the
    // `SelectionStats` event and cleared by the view on a single cell. Also the
    // live perf snapshot pumped from `perf_pub` below.
    let sel_stats = RwSignal::new(None::<trame_chrome::SelStats>);
    // Toile selection aggregates (toile-status-bar): element tally + world bounds,
    // mirrored from `ToileEvent::SelectionStats`. Cleared to `None` on an empty
    // selection so the band shows only the zoom chip.
    let sel_stats_toile = RwSignal::new(None::<toile_chrome::SelStats>);
    // Presenter overlay panel signals (toile-presentation), mirrored from the
    // enriched `SlideChanged` + `PresentModeChanged` events by the per-frame
    // relay. Created here (like `sel_stats_toile`) and passed to the shell.
    let present_toile = RwSignal::new(false);
    let presenter_index = RwSignal::new(0usize);
    let presenter_count = RwSignal::new(0usize);
    let presenter_notes = RwSignal::new(String::new());
    let presenter_frame = RwSignal::new(None::<ambre_bus::app::toile::ElementId>);
    // "Follow presenter" role signals (toile-presentation), mirrored from the
    // `PresentBroadcastChanged` / `FollowingChanged` events and from a peer's
    // broadcasting presence. `presenter_peer_toile` is the latest peer seen
    // broadcasting a slide; it drives the panel's Follow affordance.
    let broadcasting_toile = RwSignal::new(false);
    let following_toile = RwSignal::new(None::<ambre_bus::UserId>);
    let presenter_peer_toile = RwSignal::new(None::<ambre_bus::UserId>);
    let perf_snap = RwSignal::new(ambre_canvas::PerfSnapshot::default());

    // Sheet-tab strip model (`trame-sheets`): mirrored from `TrameEvent::SheetList`
    // by the relay below and fed to the DOM `SheetTabs` slot (off the canvas now).
    let sheets = RwSignal::new(Vec::<(String, String)>::new());
    let active_sheet = RwSignal::new(String::new());
    // Grid zoom factor selected in the status-bar menu (`trame-status-bar`),
    // polled by the canvas view each frame. Defaults to 100%.
    let zoom = RwSignal::new(1.0_f64);

    // Context-menu scope (`trame-context-menu`): the view sets it per right-click
    // (cell vs header) before the DOM `contextmenu` listener opens the menu. The
    // shell derives the menu layout from it. Created here (like the formula
    // signals) so both the view and the Leptos shell share the one signal.
    let ctx_scope = RwSignal::new(trame_chrome::CtxScope::Cell);
    // Active-filter flag (`trame-shared-filter`), mirrored from the Logic thread's
    // FilterApplied/FilterCleared. Drives the context menu's dynamic filter entry.
    let filter_active = RwSignal::new(false);

    // Toile context-menu bridge (`toile-menus`): the canvas view publishes the
    // selection scope on each right-click. The DOM menu reads it to build its
    // entries. It writes the chosen `CommandId` back for the view to resolve
    // against its (UI-only) selection. Created here so the view and the Leptos
    // shell share the one pair of signals, exactly like Trame's `ctx_scope`.
    let toile_ctx_scope = RwSignal::new(ambre_toile::ui::menu::CtxScope::default());
    let toile_menu_cmd = RwSignal::new(None::<ambre_chrome::commands::CommandId>);
    // Toile active tool (`toile-tools`): shared by the DOM tool palette,
    // the keyboard tool chords and the canvas view, exactly like the context-menu
    // pair above. Starts on the default tool (Select).
    let toile_active_tool = RwSignal::new(ambre_toile::ui::Tool::default());
    // Toile canvas cursor (`toile-tools`): the view publishes the
    // position-dependent cursor here (directional resize on a selection edge, grab
    // on the rotate handle). `None` falls back to the tool's base cursor.
    let toile_cursor = RwSignal::new(None::<std::borrow::Cow<'static, str>>);
    // Toile style inspector (`toile-styles`): the view publishes the selection's
    // common style into `toile_style_model`. It drains a whole-style edit from
    // `toile_restyle`, exactly like the context-menu pair above.
    let toile_style_model = RwSignal::new(None::<toile_chrome::StyleModel>);
    let toile_restyle = RwSignal::new(None::<ambre_bus::app::toile::ElementStyle>);
    // Floating selection toolbar (`toile-styles`): the view republishes its placement
    // into `toile_float_bar` each frame. It drains single-field style edits from
    // `toile_quick_style`, mirroring the inspector's model / restyle pair.
    let toile_float_bar = RwSignal::new(None::<toile_chrome::FloatBar>);
    let toile_quick_style = RwSignal::new(None::<toile_chrome::StyleEdit>);
    // Comments (`toile-comments`): the panel and the drop composer read what
    // the view published. They write back one intent at a time, the same seam
    // as the inspector's model / restyle pair.
    let toile_comments = toile_chrome::CommentSignals {
        panel: RwSignal::new(false),
        list: RwSignal::new(Vec::new()),
        open: RwSignal::new(None),
        draft: RwSignal::new(None),
        intent: RwSignal::new(None),
    };
    // The deck (`toile-presentation`): the panel renders the frames the view
    // read back and answers with one order edit at a time.
    let toile_deck = toile_chrome::DeckSignals {
        panel: RwSignal::new(false),
        view: RwSignal::new(Default::default()),
        intent: RwSignal::new(None),
    };
    // Interop (`toile-import-export`): the shell reads the file, the view lands
    // it where the user dropped it.
    let toile_dropped_file = RwSignal::new(None::<toile_chrome::DroppedFile>);
    // Embeds (`toile-embeds`): the shell owns the address dialog and the document
    // picker, the view lands the answer on the board.
    let toile_placed_embed = RwSignal::new(None::<toile_chrome::PlacedEmbed>);
    // The link pair (`toile-links`): the chooser's answer goes down to the view,
    // and a follow the board cannot perform comes back up to the shell.
    let toile_set_link = RwSignal::new(None::<toile_chrome::LinkEdit>);
    let toile_followed_link = RwSignal::new(None::<toile_chrome::ShellNav>);

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
    // the local restore. That restore must materialise the doc with the right
    // type if it wins the startup race against `OpenDocument`.
    let boot_doc_typed = boot_route.doc();
    let boot_doc = boot_doc_typed.map(|(_, id)| id);
    // Cross-type in-session navigation reloads the page (the `DocSwitcher` only
    // swaps the id within one `AppView`). So the booted app kind is fixed here,
    // and gates both the render view (below) and the app-specific chrome (Find
    // panel, keymap). Toile binds its own, smaller chord set.
    let boot_is_toile = matches!(boot_doc_typed, Some((ambre_bus::DocType::Toile, _)));
    let boot_is_prose = matches!(boot_doc_typed, Some((ambre_bus::DocType::Prose, _)));
    // The status bar slot the Prose view fills with its five figures.
    let prose_figures = RwSignal::new(None::<[String; 5]>);
    let prose_signals = host::ProseSignals::new();
    if boot_is_toile {
        toile_chrome::bind_keys(&mut keymap);
    } else if boot_is_prose {
        ambre_prose::commands::bind_keys(&mut keymap);
    } else {
        trame_chrome::bind_keys(&mut keymap);
    }
    // The canvas needs a document from frame 0. Boot onto the routed one, or a
    // throwaway id on the landing route. The canvas is hidden there, and the
    // route effect swaps it to the real document on the first navigation.
    let initial_doc = boot_doc.unwrap_or_else(DocumentId::new);

    // Spawn the Logic thread (shares this instance's memory). Owns the UI/Sync
    // halves of its queues. The Sync-bound half (`logic_to_sync_tx`) is relayed
    // to the Sync worker by the per-frame loop, and `sync_to_logic_tx` is fed by
    // the worker's onmessage handler.
    wasm_thread::Builder::new()
        .name("logic".to_owned())
        .spawn(move || {
            ambre_logic::run(
                logic_bell,
                ui_to_logic_rx,
                logic_to_ui_tx,
                logic_to_sync_tx,
                sync_to_logic_rx,
                logic_to_persist_tx,
            );
        })
        .expect("spawn Logic thread");

    // Eagerly fetch the base fonts and seed the Logic thread's PDF-export
    // FontContext (`toile-pdf-export`). The Logic worker cannot fetch: it blocks
    // on its inbound selector and has no browser event loop. So the Main thread,
    // which has both, fetches on its behalf and ships the raw bytes across as
    // `LogicCommand::LoadFonts`. Fire-and-forget on a `spawn_local`: export
    // happens far later, and a slow fetch must not stall boot.
    {
        let fonts_tx = ui_to_logic_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let faces = ambre_canvas::fonts::fetch_base_font_faces().await;
            let _ = fonts_tx.try_send(LogicCommand::LoadFonts { faces });
        });
    }

    // Dev-only multi-user testing: `?user=<name>` in the page URL authenticates as
    // that user (created on the fly server-side, keys derived deterministically
    // from the name). Lets several browser profiles collaborate on one machine
    // without OIDC. A prod server rejects the resulting `DevLogin` (DEV_MODE off).
    // The engine lives once now, so this fires only for the boot URL; the landing
    // dev-login field dispatches `DevLogin` directly thereafter (no reload).
    //
    // `?user=` wins. Otherwise fall back to the dev user remembered from a prior
    // session (localStorage). A refresh of a `/trame/<id>` deep-link carries no
    // query string, and still re-establishes the session. The dev keys are
    // re-derived deterministically from the name, so no key material is stored --
    // only the username. (Real-auth refresh-token persistence is Part D.)
    //
    // A return from the identity provider comes first: the page booted on it,
    // and the Sync worker exchanges it for the session (`CHROME-096`). It is
    // read once, and only if this tab started it (`CHROME-101`).
    if let Some(ret) = signin::take_boot_return() {
        tracing::info!("Provider return for '{}'", ret.provider);
        let _ = ui_to_logic_tx.try_send(LogicCommand::Rest {
            id: RequestId::next(),
            cmd: RestCommand::ProviderReturn {
                provider: ret.provider,
                code: ret.code,
                state: ret.state,
            },
        });
    } else if let Some(username) = dev_user_from_url().or_else(stored_dev_user) {
        tracing::info!("Dev login requested for user '{username}'");
        remember_dev_user(&username);
        let _ = ui_to_logic_tx.try_send(LogicCommand::Rest {
            id: RequestId::next(),
            cmd: RestCommand::DevLogin { username },
        });
    }

    // Spawn the Sync worker: a SEPARATE wasm instance in its own Web Worker. Its
    // onmessage decodes `LogicEvent`s into `sync_to_logic_tx` (-> Logic). Kept
    // alive for the whole session by being captured in the relay closure.
    //
    // `sync_start` gates the Logic->Sync relay: the worker installs its inbound
    // onmessage only at the end of its async wasm bootstrap. Any command posted
    // before then is dropped. The relay holds commands (they wait in
    // `logic_to_sync_rx`) until the worker announces itself (`SYNC-057`).
    // Without it the boot `DevLogin`, enqueued above before this spawn, raced
    // the bootstrap and was lost whenever the worker came up slowly.
    let sync_start = Rc::new(RefCell::new(ambre_bus::start::StartGate::new()));
    let sync_worker = spawn_sync_worker(Rc::clone(&sync_inbox), Rc::clone(&sync_start));

    // The canvas document-swap handle is created by `run_with` below (after the
    // chrome mounts the `<canvas>`); the route effect reads it through this slot.
    let switcher_slot: Rc<RefCell<Option<DocSwitcher>>> = Rc::new(RefCell::new(None));

    // ---- Mount the Leptos shell (once). Reactive state is created inside the
    // mount owner and pulled back out via slots for the imperative relay below. -
    let store_slot: Rc<RefCell<Option<CommandStates>>> = Rc::new(RefCell::new(None));
    let toile_store_slot: Rc<RefCell<Option<CommandStates>>> = Rc::new(RefCell::new(None));
    let commands_slot: Rc<RefCell<Option<shell::CommandPath>>> = Rc::new(RefCell::new(None));
    let anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>> =
        Rc::new(RefCell::new(None));
    let toasts_slot: Rc<RefCell<Option<Toasts>>> = Rc::new(RefCell::new(None));
    let user_slot: Rc<RefCell<Option<RwSignal<Option<String>>>>> = Rc::new(RefCell::new(None));
    let docs_slot: Rc<RefCell<Option<RwSignal<Vec<DocItem>>>>> = Rc::new(RefCell::new(None));
    let members_slot: Rc<RefCell<Option<RwSignal<Vec<MemberInfo>>>>> = Rc::new(RefCell::new(None));
    let devmode_slot: Rc<RefCell<Option<RwSignal<Option<bool>>>>> = Rc::new(RefCell::new(None));
    let current_doc_slot: Rc<RefCell<Option<RwSignal<Option<DocumentId>>>>> =
        Rc::new(RefCell::new(None));
    let relays_slot: Rc<RefCell<Option<(trame_chrome::TrameRelay, toile_chrome::ToileRelay)>>> =
        Rc::new(RefCell::new(None));
    let style_slot: Rc<RefCell<Option<trame_chrome::StyleState>>> = Rc::new(RefCell::new(None));
    let chrome_status_slot: Rc<RefCell<Option<RwSignal<ChromeStatus>>>> =
        Rc::new(RefCell::new(None));

    // The Paste-button signal: the shell decodes an async clipboard read and
    // hands the transport to the host, which routes it to the active view
    // (`UI-THREAD-024`). `Send`, so it crosses the Router's Send boundary.
    // Bounded like every link: one paste per gesture, drained each frame.
    let (paste_tx, paste_rx) = conduit::local::<ClipboardPaste>(CHANNEL_CAPACITY);

    mount_shell(
        ui_to_logic_tx.clone(),
        paste_tx,
        raf.clone(),
        theme_ctl.clone(),
        boot_doc,
        Rc::clone(&store_slot),
        Rc::clone(&toile_store_slot),
        Rc::clone(&anchor_slot),
        Rc::clone(&toasts_slot),
        Rc::clone(&user_slot),
        Rc::clone(&docs_slot),
        Rc::clone(&members_slot),
        Rc::clone(&devmode_slot),
        Rc::clone(&current_doc_slot),
        Rc::clone(&relays_slot),
        boot_is_toile,
        boot_is_prose,
        prose_figures,
        prose_signals,
        Rc::clone(&style_slot),
        Rc::clone(&chrome_status_slot),
        formula_ref,
        formula_text,
        formula_action,
        tofu_prompt,
        tofu_confirm,
        invite_role,
        me,
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
        toile_ctx_scope,
        toile_menu_cmd,
        toile_active_tool,
        toile_cursor,
        toile_style_model,
        toile_restyle,
        toile_float_bar,
        toile_quick_style,
        toile_comments,
        toile_deck,
        toile_dropped_file,
        toile_placed_embed,
        toile_set_link,
        toile_followed_link,
        sheets,
        active_sheet,
        Rc::clone(&commands_slot),
    );
    let commands = commands_slot
        .borrow()
        .clone()
        .expect("command path created during mount");

    let store = store_slot
        .borrow()
        .clone()
        .expect("store created during mount");
    let toile_store = toile_store_slot
        .borrow()
        .clone()
        .expect("Toile store created during mount");
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
    // Each application's relay, read back for the per-frame relay, which hands
    // it every event (`ARCH-018`).
    let (trame_relay, toile_relay) = relays_slot.borrow().expect("relays created during mount");
    // Formatting-toolbar style state (`trame-cell-styles`), read back for the
    // per-frame relay, which hands it every event.
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
    install_command_keys(keymap, commands.clone());

    // Local persistence: open the IndexedDB store on this (Main) thread, where
    // the event loop is live. On open, restore the BOOT document's snapshot (if
    // any) so an edit survives a refresh without a network round-trip. The store
    // is shared with the per-frame relay below, which saves snapshots Logic emits.
    let persist_store: Rc<RefCell<Option<local_store::LocalStore>>> = Rc::new(RefCell::new(None));
    // Set when the store will never open, so the writes waiting for it go.
    let persist_gone = Rc::new(std::cell::Cell::new(false));
    {
        let store_slot = Rc::clone(&persist_store);
        let gone = Rc::clone(&persist_gone);
        let restore_tx = ui_to_logic_tx.clone();
        let full_tx = ui_to_logic_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let store = match ambre_logic::persistence::PersistenceStore::open().await {
                Ok(Ok(store)) => store,
                Ok(Err(e)) => {
                    tracing::warn!("Persistence: store unavailable ({e})");
                    gone.set(true);
                    return;
                }
                Err(_) => {
                    tracing::warn!("Persistence: open channel dropped");
                    gone.set(true);
                    return;
                }
            };
            if let Some((doc_type, document_id)) = boot_doc_typed {
                let snapshot = store
                    .load_snapshot(*document_id.as_bytes())
                    .await
                    .ok()
                    .flatten();
                let deltas = store
                    .load_deltas(*document_id.as_bytes())
                    .await
                    .unwrap_or_default();
                let sessions = store
                    .load_sessions(*document_id.as_bytes())
                    .await
                    .unwrap_or_default();
                // Sessions alone restore nothing: without local operations
                // there is nothing of theirs left to send.
                if snapshot.is_some() || !deltas.is_empty() {
                    tracing::info!(
                        "Persistence: restoring doc ({} snapshot bytes, {} deltas, {} sessions)",
                        snapshot.as_ref().map_or(0, Vec::len),
                        deltas.len(),
                        sessions.len(),
                    );
                    let _ = restore_tx.try_send(LogicCommand::RestoreLocalSnapshot {
                        document_id,
                        doc_type,
                        snapshot,
                        deltas,
                        sessions,
                    });
                }
            }
            *store_slot.borrow_mut() = Some(local_store::LocalStore::open(store, full_tx).await);
        });
    }

    // The single UI-thread dispatcher, pumped once per frame regardless of route.
    // It runs the persist drain, the Logic->Sync relay and the anchor mirror. It
    // also runs the Logic->UI fan-out: command state to the chrome, session/list
    // to the landing, content to the view.
    let anchor_pub_pump = anchor_pub.clone();
    let perf_pub_pump = perf_pub.clone();
    let filter_open_pump = filter_open_pub.clone();
    let busy_pump = busy_pub.clone();
    let notice_pump = notice_pub.clone();
    let persist_store_frame = Rc::clone(&persist_store);
    let persist_gone_frame = Rc::clone(&persist_gone);
    let mut persist_waiting: Vec<(DocumentId, ambre_logic::PersistWrite)> = Vec::new();
    let mut last_anchor: Option<(f64, f64)> = None;
    let mut last_busy = false;
    let mut last_perf_push_ms: f64 = 0.0;
    let mut list_requested = false;
    let relay_tx = ui_to_logic_tx.clone();
    // The relay's view of the child fan-out registry: read to route a
    // nested document's events, pruned when a child receiver has been dropped.
    let child_routes_relay = Rc::clone(&child_routes);
    let child_backlog_relay = Rc::clone(&child_backlog);

    // SPA route binding, polled each frame on the main thread (where the `!Send`
    // canvas handle lives, so it stays OUT of the Leptos tree). The client URL is
    // the single source of truth: `leptos_router` pushState navigations and
    // browser back/forward both land here. On a change we close the previous
    // document, open the next, and point the persistent canvas at it. We then
    // gate the rAF render: paint the doc, or freeze it behind the landing
    // overlay. We publish the active document, so every command trigger targets
    // it.
    let route_switcher = Rc::clone(&switcher_slot);
    let route_tx = ui_to_logic_tx.clone();
    let route_raf = raf.clone();
    let mut prev_route: Option<route::Route> = None;
    let mut last_title: Option<String> = None;

    let pre_frame = move || {
        let cur_route = route::current();
        let cur_doc = cur_route.doc();
        if let Some(step) = route::step(prev_route.as_ref(), &cur_route) {
            if let Some(document_id) = step.close {
                let _ = route_tx.try_send(LogicCommand::CloseDocument { document_id });
            }
            if let Some((doc_type, document_id)) = step.open {
                let _ = route_tx.try_send(LogicCommand::OpenDocument {
                    document_id,
                    doc_type,
                });
            }
            if step.close.is_some() || step.open.is_some() {
                current_doc.set(step.show);
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
            match step.show {
                Some(document_id) => {
                    if let Some(sw) = route_switcher.borrow().as_ref() {
                        sw.set_document(document_id);
                    }
                    route_raf.set_away(false);
                }
                // Freeze the canvas behind the landing overlay (the relay below
                // keeps running -- only the render is gated).
                None => route_raf.set_away(true),
            }
            prev_route = Some(cur_route);
        }

        // Browser tab title (`route::tab_title`). Recomputed each frame so a
        // rename or a late-arriving document list updates the tab; the DOM
        // write happens only when the title changes.
        let title = match cur_doc {
            Some((doc_type, document_id)) => {
                let name = docs.with_untracked(|list| {
                    list.iter()
                        .find(|d| d.document_id == document_id)
                        .and_then(|d| d.name.clone())
                });
                route::tab_title(Some((doc_type, name.as_deref())))
            }
            None => route::tab_title(None),
        };
        if last_title.as_deref() != Some(title.as_str()) {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                doc.set_title(&title);
            }
            last_title = Some(title);
        }

        // Persist Logic -> local store. Writes wait here until the store is
        // open: a delta dropped would be missing from every restore until the
        // next snapshot. Best-effort; the put() commits as the event loop idles.
        persist_waiting.extend(logic_to_persist_rx.try_iter());
        if persist_gone_frame.get() {
            // A store that never opens holds no copy: each reopen waiting on
            // one is answered empty, and fetches from the relay (`RESID-012`).
            for (document_id, write) in persist_waiting.drain(..) {
                if let ambre_logic::PersistWrite::ReadBack { doc_type } = write {
                    let _ = relay_tx.try_send(LogicCommand::RestoreLocalSnapshot {
                        document_id,
                        doc_type,
                        snapshot: None,
                        deltas: Vec::new(),
                        sessions: Vec::new(),
                    });
                }
            }
        }
        if let Some(local) = persist_store_frame.borrow().as_ref() {
            local.tick();
            for (document_id, write) in persist_waiting.drain(..) {
                match write {
                    // Read after every write queued before it, so the copy
                    // holds what the document had when it left (`RESID-010`).
                    ambre_logic::PersistWrite::ReadBack { doc_type } => {
                        let store = local.store().clone();
                        let tx = relay_tx.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let key = *document_id.as_bytes();
                            let snapshot = store.load_snapshot(key).await.ok().flatten();
                            let deltas = store.load_deltas(key).await.unwrap_or_default();
                            let sessions = store.load_sessions(key).await.unwrap_or_default();
                            let _ = tx.try_send(LogicCommand::RestoreLocalSnapshot {
                                document_id,
                                doc_type,
                                snapshot,
                                deltas,
                                sessions,
                            });
                        });
                    }
                    // Each write passes the store's ledger, which keeps it
                    // within its budget and meets the quota (`RESID-002`).
                    write => local.apply(document_id, write),
                }
            }
        }

        // TOFU: a confirmed prompt -> pin the fetched key in `trusted_keys` and
        // fire phase 2 of the invite (`ambre-spec-key-verification.md`). Handled here
        // rather than in `AppRoot` because the trusted-key store is Main-thread
        // owned (this relay). Writing the pin before dispatch is normative.
        if let Some(p) = tofu_confirm.get_untracked() {
            tofu_confirm.set(None);
            tofu::confirm(
                &p,
                invite_role.get_untracked(),
                |user, key| {
                    if let Some(local) = persist_store_frame.borrow().as_ref() {
                        local.store().save_trusted_key(user, key);
                    }
                },
                |cmd| {
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd,
                    });
                },
            );
        }

        // Relay Logic -> Sync: drain commands and postMessage them to the Sync
        // worker (postcard, big payloads transferred zero-copy). `sync_worker` is
        // captured here, which is what keeps the worker alive for the session.
        // Held back until the worker announces itself (`sync_start`): commands
        // stay in `logic_to_sync_rx`, in order, through the worker's bootstrap.
        // Sync -> Logic: re-offer what the channel refused on an earlier turn.
        sync_inbox.borrow_mut().flush();
        relay_pressure(&sync_inbox, &sync_worker);

        sync_start
            .borrow()
            .drain(&logic_to_sync_rx, |cmd| match postcard::to_allocvec(&cmd) {
                Ok(bytes) => post_to_worker(&sync_worker, &bytes),
                Err(e) => tracing::error!("[UI->Sync] failed to encode SyncCommand: {e}"),
            });

        // Re-offer what a child's queue could not take on an earlier turn
        // (`TOILE-UI-340`). Ordered, and ahead of the fresh stream, so a child
        // never sees a later delta before an earlier one.
        flush_child_backlog(&child_routes_relay, &child_backlog_relay);

        // Mark the turn that takes Logic's answers off the channel: the mark
        // splits a keystroke's confirmation between the relay and the view.
        let mut relayed = false;
        while let Ok(ev) = logic_to_ui_rx.try_recv() {
            if !relayed {
                relayed = true;
                host::mark(ambre_prose::probe::RELAY);
            }
            // Fan a nested child's stream out to its own view queues, and skip
            // every top-level chrome mirror below. A child's events must not
            // drive the parent's formula bar, zoom chip or status band. Only a
            // document that is a LIVE child is diverted; the top-level document's
            // events (and untagged service events) fall through unchanged.
            if let Some(id) = ev.routed_document_id()
                && child_routes_relay.borrow().contains_key(&id)
            {
                send_to_child(&child_routes_relay, &child_backlog_relay, id, ev);
                continue;
            }
            // Mirror authoritative command-toggle state into the chrome store.
            // The active app owns the event->toggle mapping; the launcher names
            // no content event variant.
            trame_chrome::mirror_event(&store, &ev);
            // Mirror Toile's Undo/Redo availability into its own command store,
            // so the menu bar disables the two rows from the Logic thread's
            // `UndoState` alone (`UNDO-006`).
            toile_chrome::mirror_event(&toile_store, &ev);
            // Prose's history availability, into the signals its context
            // menu reads (`UNDO-006`).
            prose_signals.mirror_event(&ev);
            // Mirror the active cell's rich style into the formatting toolbar
            // (`trame-cell-styles`). A no-op for non-style events.
            trame_chrome::apply_style_event(&style_state, &ev);
            // Surface Trame's user-facing notifications (rejections, validation
            // errors, sheet-lifecycle errors, sort/filter completion) as toasts.
            // The app owns the event->toast mapping; the launcher names no variant.
            trame_chrome::notify_event(&toasts, &ev);
            // Surface Toile's interop outcomes -- the lossy-import report, a
            // refused file, a finished export -- as toasts (`TOILE-IO-022`). The
            // app owns the event->toast mapping; the launcher names no variant.
            toile_chrome::notify_event(&toasts, &ev);
            // Prose's conversion reports, the same way (`PROSE-IO-053`).
            ambre_prose::chrome::notify_event(&toasts, &ev);
            // Each application fills its own panels from its own events
            // (`ARCH-018`). The launcher names no variant.
            trame_relay.absorb(&ev);
            toile_relay.absorb(&ev);
            match &ev {
                // Non-fatal sync warnings surface as a transient warning toast.
                // They never reach the fatal path. The session keeps running and
                // the CRDT reconverges: a dropped/quarantined update is re-fetched
                // via RequestMissing once its parent snapshot lands.
                UiEvent::SyncWarning { code, .. } => toasts.warning(sync_warning_message(*code)),
                // The relay moved to a newer wire version: syncing stays off
                // until the page reloads onto the new client (`SYNC-137`).
                UiEvent::UpdateRequired => toasts.warning(
                    "A new version of Ambre is available. Reload the page to keep syncing.",
                ),
                // Session bootstrap (dev `?user=` or OIDC): show the user, and
                // request the document list once for the landing overlay.
                UiEvent::Rest {
                    evt:
                        ambre_bus::RestEvent::SessionReady {
                            user_id,
                            display_name,
                            ..
                        },
                    ..
                } => {
                    username.set(Some(display_name.clone()));
                    me.set(Some(*user_id));
                    if !list_requested {
                        list_requested = true;
                        let _ = relay_tx.try_send(LogicCommand::Rest {
                            id: RequestId::next(),
                            cmd: RestCommand::ListDocuments,
                        });
                    }
                }
                // The provider's address: send the browser there (`CHROME-095`).
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::ProviderAddress { url, state },
                    ..
                } => {
                    signin::follow_address(url, state, |url| {
                        if let Some(window) = web_sys::window() {
                            let _ = window.location().set_href(url);
                        }
                    });
                }
                // Signed in with the provider: the Sync worker holds the tokens,
                // and the keys are still to open or to set up (`CHROME-096`).
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::ProviderSignedIn { keys_stored, .. },
                    ..
                } => {
                    tracing::info!("Provider sign-in done (keys stored: {keys_stored})");
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
                // The user left a document (`SYNC-141`), and the worker has
                // closed it. A view still on it goes back to the launcher, and
                // the list drops its card.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::DocumentLeft { document_id },
                    ..
                } => {
                    docs.update(|list| list.retain(|d| d.document_id != *document_id));
                    if current_doc.get_untracked() == Some(*document_id)
                        && let Some(window) = web_sys::window()
                    {
                        let _ = window.location().set_href("/");
                    }
                }
                // A sealed file came back open (`DOCFILE-014`): what it holds
                // is a whole plaintext container, so the ordinary open path
                // takes it from here. Version recovery, the type check and the
                // local store are the same steps a plaintext file walks.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::DocFileOpened { plaintext },
                    ..
                } => platform::open_plaintext_container(plaintext.to_vec(), toasts),
                // One message for two causes, and that is the contract rather
                // than laziness. The tag cannot tell a wrong passphrase from a
                // modified file, and a message that named one would be a guess
                // the user trusts (`DOCFILE-016`).
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::DocFileOpenFailed,
                    ..
                } => toasts.error(
                    "That file did not open. The passphrase may be wrong, or the file may be damaged.",
                ),
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
                // TOFU phase 1 answer: classify the fetched key against the
                // pinned one. Trusted (same key) -> dispatch phase 2 straight away;
                // New (no pin) or Changed (divergence) -> raise the modal prompt.
                UiEvent::Rest {
                    evt:
                        ambre_bus::RestEvent::InviteeKey {
                            document_id,
                            user_id,
                            display_name,
                            email,
                            exchange_pub_key,
                            fingerprint,
                        },
                    ..
                } => {
                    let prompt = TofuPrompt {
                        document_id: *document_id,
                        user_id: *user_id,
                        display_name: display_name.clone(),
                        email: email.clone(),
                        exchange_pub_key: *exchange_pub_key,
                        fingerprint: fingerprint.clone(),
                        changed: false,
                    };
                    // Read the pin off-thread (async IndexedDB); never hold the
                    // store borrow across the await.
                    let pin_rx = persist_store_frame
                        .borrow()
                        .as_ref()
                        .map(|s| s.store().load_trusted_key(*prompt.user_id.as_bytes()));
                    let relay = relay_tx.clone();
                    let role_sig = invite_role;
                    let prompt_sig = tofu_prompt;
                    wasm_bindgen_futures::spawn_local(async move {
                        let pinned = match pin_rx {
                            Some(rx) => rx.await.ok().flatten(),
                            None => None,
                        };
                        match tofu::classify(prompt, pinned, role_sig.get_untracked()) {
                            // Trusted: unchanged key -> proceed without interrupting.
                            tofu::Next::Invite(cmd) => {
                                let _ = relay.try_send(LogicCommand::Rest {
                                    id: RequestId::next(),
                                    cmd,
                                });
                            }
                            // New or changed: the modal decides.
                            tofu::Next::Ask(prompt) => prompt_sig.set(Some(prompt)),
                        }
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
                UiEvent::DocumentReady { document_id, .. }
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
                // Relayed service failures (for example unknown invitee) surface as a
                // warning toast so the share action gives feedback.
                UiEvent::ApiError { message, .. } => toasts.warning(message.clone()),
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

        // Say that the view stopped taking input, and say when it resumes
        // (`BUS-016`). The view restates the level every frame, so this reads the
        // edge: one notice per episode rather than one per frame. Without it the
        // user meets a document answering nothing and reads it as a freeze.
        let busy = busy_pump.is_busy();
        if busy != last_busy {
            last_busy = busy;
            if busy {
                toasts.warning("The document is catching up. Your edit is queued.");
            } else {
                toasts.info("The document is up to date.");
            }
        }

        // Raise what the view could not show on the board.
        for message in notice_pump.take() {
            toasts.warning(message);
        }

        // Open the shared filter popover when the canvas raised the one-shot intent
        // (`trame-shared-filter`): a click on an in-canvas autofilter button. The
        // view already moved the active cell onto the button's column, so the anchor
        // mirrored just above points at it. It asked nothing else. This runs the
        // exact body of the context-menu `FILTER_COLUMN` branch, so the two entry
        // points share one filter code path.
        if filter_open_pump.take()
            && let Some(trame) = commands.handler.trame()
        {
            commands.send(trame.open_filter());
        }

        // Mirror the view's perf snapshot into the status-bar signal, throttled
        // to ~2 Hz. The old wgpu HUD refreshed every 500 ms too, and a per-frame
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
    // and drives the Trame view through the rAF loop. It returns the
    // document-swap handle the route effect uses on navigation.
    let view_tx_logic = ui_to_logic_tx.clone();
    let formula_ref_tv = formula_ref;
    let formula_text_tv = formula_text;
    let formula_action_tv = formula_action;

    // Child-view factory for nested `doc_embed`s. `FrameHost` cannot build an
    // `AppView` (it is app-agnostic, and the chrome signals need a reactive owner
    // it does not have), so the launcher supplies this. Each call registers a
    // demultiplexed Logic->UI queue and returns a view in embedded mode (it does
    // not grab the text pipe). The chrome signals below are throwaway: no DOM
    // reads them, so every child shares one dead set. It is created here, in the
    // launcher's leptos owner, where `RwSignal::new` is valid.
    let child_to_logic = ui_to_logic_tx.clone();
    let child_routes_factory = Rc::clone(&child_routes);
    let child_theme_ctl = theme_ctl.clone();
    // An embedded child sends on its host board's UI->Logic queue, so the two
    // share one busy level rather than each holding a level nobody reads.
    let child_busy_pub = busy_pub.clone();
    let child_notice_pub = notice_pub.clone();
    let ch_toile_ctx_scope = RwSignal::new(ambre_toile::ui::menu::CtxScope::default());
    let ch_toile_menu_cmd = RwSignal::new(None::<CommandId>);
    let ch_toile_active_tool = RwSignal::new(ambre_toile::ui::Tool::default());
    let ch_toile_cursor = RwSignal::new(None::<std::borrow::Cow<'static, str>>);
    let ch_toile_zoom = RwSignal::new(1.0_f64);
    let ch_toile_sel_stats = RwSignal::new(None::<toile_chrome::SelStats>);
    let ch_toile_style_model = RwSignal::new(None::<toile_chrome::StyleModel>);
    let ch_toile_restyle = RwSignal::new(None::<ambre_bus::app::toile::ElementStyle>);
    let ch_toile_float_bar = RwSignal::new(None::<toile_chrome::FloatBar>);
    let ch_toile_quick_style = RwSignal::new(None::<toile_chrome::StyleEdit>);
    let ch_toile_comments = toile_chrome::CommentSignals {
        panel: RwSignal::new(false),
        list: RwSignal::new(Vec::new()),
        open: RwSignal::new(None),
        draft: RwSignal::new(None),
        intent: RwSignal::new(None),
    };
    let ch_toile_deck = toile_chrome::DeckSignals {
        panel: RwSignal::new(false),
        view: RwSignal::new(Default::default()),
        intent: RwSignal::new(None),
    };
    let ch_toile_dropped_file = RwSignal::new(None::<toile_chrome::DroppedFile>);
    let ch_toile_placed_embed = RwSignal::new(None::<toile_chrome::PlacedEmbed>);
    let ch_toile_set_link = RwSignal::new(None::<toile_chrome::LinkEdit>);
    let ch_toile_followed_link = RwSignal::new(None::<toile_chrome::ShellNav>);
    let ch_formula_ref = RwSignal::new(String::new());
    let ch_formula_text = RwSignal::new(String::new());
    let ch_formula_action = RwSignal::new(None::<FormulaAction>);
    let ch_tr_sel_stats = RwSignal::new(None::<trame_chrome::SelStats>);
    let ch_tr_ctx_scope = RwSignal::new(trame_chrome::CtxScope::Cell);
    let ch_tr_zoom = RwSignal::new(1.0_f64);
    let make_child_fn = move |document_id: DocumentId,
                              doc_type: ambre_bus::DocType,
                              size: ambre_draw::Size,
                              text_input: ambre_canvas::TextInput|
          -> AppView {
        // Demultiplexed Logic->UI queue for this child (registered for the relay
        // fan-out; dropped with the child when its embed leaves the cull set).
        let (child_tx, child_rx) = conduit::local::<UiEvent>(CHANNEL_CAPACITY);
        child_routes_factory
            .borrow_mut()
            .entry(document_id)
            .or_default()
            .push(child_tx);
        match doc_type {
            ambre_bus::DocType::Toile => AppView::Toile(ToileView::new(
                document_id,
                child_to_logic.clone(),
                child_rx,
                size.width,
                size.height,
                text_input,
                true,
                ambre_canvas::PerfPublisher::new(),
                child_busy_pub.clone(),
                child_notice_pub.clone(),
                child_theme_ctl.clone(),
                ch_toile_ctx_scope,
                ch_toile_menu_cmd,
                ch_toile_active_tool,
                ch_toile_cursor,
                ch_toile_zoom,
                ch_toile_sel_stats,
                ch_toile_style_model,
                ch_toile_restyle,
                ch_toile_float_bar,
                ch_toile_quick_style,
                ch_toile_comments,
                ch_toile_deck,
                ch_toile_dropped_file,
                ch_toile_placed_embed,
                ch_toile_set_link,
                ch_toile_followed_link,
            )),
            // Trame, or any non-Toile embed (only Trame/Toile are paintable views;
            // a stray type falls back to Trame rather than crash the frame).
            _ => AppView::Trame(TrameView::new(
                document_id,
                child_to_logic.clone(),
                child_rx,
                size.width,
                size.height,
                text_input,
                true,
                ambre_canvas::AnchorPublisher::new(),
                ambre_canvas::PerfPublisher::new(),
                child_busy_pub.clone(),
                ambre_canvas::FilterOpenPublisher::new(),
                child_theme_ctl.clone(),
                Box::new(host::PageClock),
                ch_formula_ref,
                ch_formula_text,
                ch_formula_action,
                ch_tr_sel_stats,
                ch_tr_ctx_scope,
                ch_tr_zoom,
            )),
        }
    };

    let switcher = host::run_with(
        "ambre-canvas",
        move |w, h, text_input| {
            // Pick the view by the booted document's type. Cross-type in-session
            // navigation is out of Phase 0: the `DocSwitcher` only swaps the id
            // within one `AppView`, so a Trame<->Toile hop reloads via the route.
            if boot_is_prose {
                AppView::Prose(host::ProseView::new(
                    initial_doc,
                    view_tx_logic,
                    view_rx,
                    w,
                    h,
                    text_input,
                    theme_ctl,
                    prose_figures,
                    prose_signals,
                    busy_pub.clone(),
                    notice_pub.clone(),
                ))
            } else if boot_is_toile {
                AppView::Toile(ToileView::new(
                    initial_doc,
                    view_tx_logic,
                    view_rx,
                    w,
                    h,
                    text_input,
                    false,
                    perf_pub,
                    busy_pub,
                    notice_pub,
                    theme_ctl,
                    toile_ctx_scope,
                    toile_menu_cmd,
                    toile_active_tool,
                    toile_cursor,
                    zoom,
                    sel_stats_toile,
                    toile_style_model,
                    toile_restyle,
                    toile_float_bar,
                    toile_quick_style,
                    toile_comments,
                    toile_deck,
                    toile_dropped_file,
                    toile_placed_embed,
                    toile_set_link,
                    toile_followed_link,
                ))
            } else {
                AppView::Trame(TrameView::new(
                    initial_doc,
                    view_tx_logic,
                    view_rx,
                    w,
                    h,
                    text_input,
                    false,
                    anchor_pub,
                    perf_pub,
                    busy_pub,
                    filter_open_pub,
                    theme_ctl,
                    Box::new(host::PageClock),
                    formula_ref_tv,
                    formula_text_tv,
                    formula_action_tv,
                    sel_stats,
                    ctx_scope,
                    zoom,
                ))
            }
        },
        make_child_fn,
        pre_frame,
        raf,
        paste_rx,
        // A lazily fetched script face reaches the Logic thread over this
        // channel, the way a base face does at boot (`CANVAS-068`).
        ui_to_logic_tx.clone(),
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
