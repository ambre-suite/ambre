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

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{
    AppCommand, AppEvent, DocumentId, LogicCommand, LogicEvent, MemberInfo, RequestId, RestCommand,
    Role, SyncCommand, TrameCommand, UiEvent,
};
use ambre_chrome::commands::{CommandId, Keymap};
use ambre_chrome::{
    AppShell, CommandStates, ContextMenu, Dialog, FormulaBar, Header, StatusBar, ToastHost, Toasts,
    Toolbar,
};
// Trame-specific chrome (command wiring + toolbar / selection-toolbar markup +
// status label). The launcher stays app-agnostic and reaches into this module
// for every Trame concern -- it names no command, button, or event variant itself.
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
    /// Status-bar selection aggregates (`-ui.md` sec.13bis); `None` on a single
    /// cell. Set by the per-frame relay on `TrameEvent::SelectionStats`, cleared
    /// by the canvas view on a single-cell selection.
    sel_stats: RwSignal<Option<trame_chrome::SelStats>>,
    /// Status-bar perf snapshot (fps + frame timings), pumped from the canvas
    /// view's `PerfPublisher` by the per-frame relay (~2 Hz).
    perf_snap: RwSignal<ambre_canvas::PerfSnapshot>,
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
    // its own chords -- the launcher names none.
    let mut keymap = Keymap::new();
    trame_chrome::bind_keys(&mut keymap);

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

    // Formula bar signals (HTML input -> TrameView sync). RwSignal is Copy, so
    // we create them once and pass clones to both TrameView and the Leptos shell.
    let formula_ref = RwSignal::new(String::new());
    let formula_text = RwSignal::new(String::new());
    let formula_action = RwSignal::new(None::<FormulaAction>);

    // Status-bar signals (`-ui.md` sec.13bis): selection aggregates (set from the
    // `SelectionStats` event / cleared by the view on a single cell) and the live
    // perf snapshot pumped from `perf_pub` below.
    let sel_stats = RwSignal::new(None::<trame_chrome::SelStats>);
    let perf_snap = RwSignal::new(ambre_canvas::PerfSnapshot::default());

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
    let sync_worker = spawn_sync_worker(sync_to_logic_tx);

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
    let style_slot: Rc<RefCell<Option<trame_chrome::StyleState>>> = Rc::new(RefCell::new(None));

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
        Rc::clone(&style_slot),
        formula_ref,
        formula_text,
        formula_action,
        sel_stats,
        perf_snap,
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
    // Formatting-toolbar style state (`-ui.md` sec.5bis), read back for the
    // per-frame relay (which updates it on `TrameEvent::StyleApplied`).
    let style_state = style_slot
        .borrow()
        .expect("style state created during mount");

    // Learn whether the server enables dev login (drives the signed-out panel).
    // Unauthenticated, Main-thread fetch -- a pre-session concern.
    wasm_bindgen_futures::spawn_local(async move {
        dev_mode.set(Some(fetch_dev_mode().await));
    });

    // Keyboard: resolve registered chords and dispatch to the routed document.
    install_command_keys(keymap, current_doc, ui_to_logic_tx.clone(), find_state);

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

    let pre_frame = move || {
        let cur_route = route::current();
        if prev_route.as_ref() != Some(&cur_route) {
            let prev_doc = prev_route.as_ref().and_then(route::Route::doc);
            let cur_doc = cur_route.doc();
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
            // Browser tab title: an icon-less context, so it composes the generic
            // localized type label (`DocType::generic_label`, distinct from the
            // brand `name()`) rather than the type word -- `ambre-spec-chrome.md
            // 2.G`. The open document's plaintext name is not available here, so
            // an open doc shows the generic placeholder; the landing is "Ambre".
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                let title = match cur_doc {
                    Some((doc_type, _)) => {
                        format!("Untitled - {}", doc_type.generic_label())
                    }
                    None => "Ambre".to_owned(),
                };
                doc.set_title(&title);
            }
            prev_route = Some(cur_route);
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
        while let Ok(cmd) = logic_to_sync_rx.try_recv() {
            match postcard::to_allocvec(&cmd) {
                Ok(bytes) => post_to_worker(&sync_worker, &bytes),
                Err(e) => tracing::error!("[UI->Sync] failed to encode SyncCommand: {e}"),
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
                // The filter took effect or was cleared -> close the popover.
                UiEvent::Viewport {
                    event:
                        AppEvent::Trame(
                            ambre_bus::TrameEvent::FilterApplied { .. }
                            | ambre_bus::TrameEvent::FilterCleared,
                        ),
                    ..
                } => filter_popover.set(None),
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
            AppView::Trame(TrameView::new(
                initial_doc,
                view_tx_logic,
                view_rx,
                w,
                h,
                text_input,
                anchor_pub,
                perf_pub,
                theme_ctl,
                formula_ref_tv,
                formula_text_tv,
                formula_action_tv,
                sel_stats,
            ))
        },
        pre_frame,
        raf,
    );
    // Publish the canvas swap handle for the per-frame route poll. The poll
    // reruns every frame, so it binds the routed document as soon as this lands.
    *switcher_slot.borrow_mut() = Some(switcher);
}

/// Mount the Leptos shell onto the page body: the persistent AppShell chrome
/// (which owns the `<canvas>`) plus the landing overlay shown on `/`. All
/// reactive state is created here (the mount owner) and published via the slots
/// so the imperative engine wiring in `main` can read it back.
#[allow(clippy::too_many_arguments)]
fn mount_shell(
    to_logic: flume::Sender<LogicCommand>,
    raf: ambre_canvas::RafControl,
    theme_ctl: ambre_canvas::ThemeControl,
    boot_doc: Option<DocumentId>,
    store_slot: Rc<RefCell<Option<CommandStates>>>,
    anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>>,
    toasts_slot: Rc<RefCell<Option<Toasts>>>,
    user_slot: Rc<RefCell<Option<RwSignal<Option<String>>>>>,
    docs_slot: Rc<RefCell<Option<RwSignal<Vec<DocItem>>>>>,
    members_slot: Rc<RefCell<Option<RwSignal<Vec<MemberInfo>>>>>,
    devmode_slot: Rc<RefCell<Option<RwSignal<Option<bool>>>>>,
    current_doc_slot: Rc<RefCell<Option<RwSignal<Option<DocumentId>>>>>,
    filter_popover_slot: Rc<RefCell<Option<RwSignal<Option<trame_chrome::FilterPopoverState>>>>>,
    filter_values_slot: Rc<RefCell<Option<RwSignal<Vec<String>>>>>,
    find_state_slot: Rc<RefCell<Option<RwSignal<Option<trame_chrome::FindReplaceState>>>>>,
    find_result_slot: Rc<RefCell<Option<RwSignal<Option<(usize, usize)>>>>>,
    style_slot: Rc<RefCell<Option<trame_chrome::StyleState>>>,
    formula_ref: RwSignal<String>,
    formula_text: RwSignal<String>,
    formula_action: RwSignal<Option<FormulaAction>>,
    sel_stats: RwSignal<Option<trame_chrome::SelStats>>,
    perf_snap: RwSignal<ambre_canvas::PerfSnapshot>,
) {
    leptos::mount::mount_to_body(move || {
        // The store stays here (in the mount owner, NOT in the view): it is `Rc`
        // and the per-frame relay drives it. The shell sees only its toggle
        // signals (Send), so the `<Router>` subtree stays Send.
        let store = CommandStates::new(trame_chrome::COMMANDS);
        // Seed app-specific initial command states (Undo/Redo start disabled).
        trame_chrome::init_command_states(&store);
        let toggles = trame_chrome::Toggles::from_store(&store);
        *store_slot.borrow_mut() = Some(store);
        let anchor_pos = RwSignal::new(None::<(f64, f64)>);
        *anchor_slot.borrow_mut() = Some(anchor_pos);
        let toasts = Toasts::new();
        *toasts_slot.borrow_mut() = Some(toasts);
        let username = RwSignal::new(None::<String>);
        *user_slot.borrow_mut() = Some(username);
        let docs = RwSignal::new(Vec::<DocItem>::new());
        *docs_slot.borrow_mut() = Some(docs);
        let members = RwSignal::new(Vec::<MemberInfo>::new());
        *members_slot.borrow_mut() = Some(members);
        let dev_mode = RwSignal::new(None::<bool>);
        *devmode_slot.borrow_mut() = Some(dev_mode);
        let current_doc = RwSignal::new(boot_doc);
        *current_doc_slot.borrow_mut() = Some(current_doc);
        let filter_popover = RwSignal::new(None::<trame_chrome::FilterPopoverState>);
        *filter_popover_slot.borrow_mut() = Some(filter_popover);
        let filter_values = RwSignal::new(Vec::<String>::new());
        *filter_values_slot.borrow_mut() = Some(filter_values);
        let find_state = RwSignal::new(None::<trame_chrome::FindReplaceState>);
        *find_state_slot.borrow_mut() = Some(find_state);
        let find_result = RwSignal::new(None::<(usize, usize)>);
        *find_result_slot.borrow_mut() = Some(find_result);
        let style_state = trame_chrome::StyleState::new();
        *style_slot.borrow_mut() = Some(style_state);

        // Provide the wiring via context rather than as a prop: `<Router>`'s
        // children must be `Fn` (re-runnable), so the children closure cannot
        // consume a moved `wiring`. `AppRoot` clones it back out of context.
        provide_context(Wiring {
            to_logic,
            raf,
            theme_ctl,
            toggles,
            style_state,
            anchor_pos,
            toasts,
            username,
            docs,
            members,
            dev_mode,
            current_doc,
            filter_popover,
            filter_values,
            find_state,
            find_result,
            formula_ref,
            formula_text,
            formula_action,
            sel_stats,
            perf_snap,
        });

        view! {
            <Router>
                <AppRoot />
            </Router>
        }
    });
}

/// The single root view, rendered inside `<Router>` so it can read the client
/// route. Renders the persistent chrome + canvas and overlays the landing on the
/// `/` route. The document binding (open/close, canvas swap, rAF gate) is driven
/// OUTSIDE this tree by the per-frame route poll (see `pre_frame` in `main`),
/// because the canvas handle is `!Send` and the `<Router>` subtree must be Send.
#[component]
fn AppRoot() -> impl IntoView {
    let Wiring {
        to_logic,
        raf,
        theme_ctl,
        toggles,
        style_state,
        anchor_pos,
        toasts,
        username,
        docs,
        members,
        dev_mode,
        current_doc,
        filter_popover,
        filter_values,
        find_state,
        find_result,
        formula_ref,
        formula_text,
        formula_action,
        sel_stats,
        perf_snap,
    } = use_context::<Wiring>().expect("Wiring provided by mount_shell");

    // The reactive client route, used only to show/hide the landing overlay.
    let location = use_location();
    let is_landing = move || {
        matches!(
            route::parse(&location.pathname.get()),
            route::Route::Landing
        )
    };

    // Theme toggle drives only the button glyph; `theme_ctl` (canvas) and the
    // document `data-theme` (CSS custom properties) are the real switches.
    let theme_sig = RwSignal::new(theme_ctl.get());

    // About modal: opening it suspends the canvas rAF, closing it resumes.
    let about_open = RwSignal::new(false);
    let on_open = Callback::new({
        let raf = raf.clone();
        move |()| raf.pause()
    });
    let on_close = Callback::new({
        let raf = raf.clone();
        move |()| raf.resume()
    });

    // Share modal: same rAF suspend/resume. Opening it requests the current
    // member list so the dialog renders it; the relay fills `members` on the
    // `DocumentMembers` response.
    let share_open = RwSignal::new(false);
    let share_on_open = Callback::new({
        let raf = raf.clone();
        let to_logic = to_logic.clone();
        move |()| {
            raf.pause();
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = to_logic.try_send(LogicCommand::Rest {
                    id: RequestId::next(),
                    cmd: RestCommand::ListDocumentMembers { document_id },
                });
            }
        }
    });
    let share_on_close = Callback::new({
        let raf = raf.clone();
        move |()| raf.resume()
    });
    // Invite form state: the typed invitee (email, or a bare username in dev
    // mode) and the role to grant.
    let invitee_input = RwSignal::new(String::new());
    let invite_role = RwSignal::new(Role::Editor);

    // Canvas right-click -> context menu (the launcher owns the listener since
    // the canvas is created by the AppShell).
    let ctx_menu = RwSignal::new(None::<(f64, f64)>);
    install_canvas_context_menu(ctx_menu);

    // The single command path shared by every trigger (toolbar, selection bar,
    // context menu): a `CommandId` -> the routed document. `Callback` is `Copy`,
    // so it is handed to each app-chrome component below.
    let on_command = Callback::new({
        let tx = to_logic.clone();
        move |id: CommandId| {
            // `FILTER_COLUMN` is a UI-only intent: open the popover anchored under
            // the active cell and ask the Logic thread for the column's distinct
            // values (it resolves the active column itself). Everything else flows
            // through the shared CommandId -> AppCommand path.
            if id == trame_chrome::FIND || id == trame_chrome::REPLACE {
                // UI-only intents (sec.5.G): open the find/replace panel in the
                // matching mode. The panel emits FindNext/ReplaceAll itself.
                let mode = if id == trame_chrome::FIND {
                    trame_chrome::FindMode::Find
                } else {
                    trame_chrome::FindMode::Replace
                };
                find_state.set(Some(trame_chrome::FindReplaceState { mode }));
            } else if id == trame_chrome::FILTER_COLUMN {
                let (x, y) = client_anchor(anchor_pos.get_untracked());
                // `col` is provisional; the ColumnValues reply carries the real one.
                filter_popover.set(Some(trame_chrome::FilterPopoverState { col: 0, x, y }));
                filter_values.set(Vec::new());
                if let Some(document_id) = current_doc.get_untracked() {
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command: AppCommand::Trame(TrameCommand::RequestActiveColumnValues),
                    });
                }
            } else if id == trame_chrome::BOLD || id == trame_chrome::ITALIC {
                // Bold/Italic are part of the single style channel: toggle the
                // tracked toolbar state and emit a `SetCellStyle` patch (sec.5bis).
                let is_bold = id == trame_chrome::BOLD;
                let cur = if is_bold {
                    style_state.bold.get_untracked()
                } else {
                    style_state.italic.get_untracked()
                };
                let mut patch = ambre_bus::StylePatch::default();
                if is_bold {
                    patch.bold = Some(!cur);
                } else {
                    patch.italic = Some(!cur);
                }
                if let Some(document_id) = current_doc.get_untracked() {
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command: AppCommand::Trame(TrameCommand::SetCellStyle { patch }),
                    });
                }
            } else if id == trame_chrome::COPY || id == trame_chrome::CUT {
                // Copy/cut (sec.5.E): focus the canvas text pipe and ask the
                // browser to fire the `copy`/`cut` DOM event the host listeners
                // service (they write both the TSV and the typed payload).
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    if let Ok(Some(ta)) = doc.query_selector(".ambre-input")
                        && let Some(el) = ta.dyn_ref::<web_sys::HtmlElement>()
                    {
                        let _ = el.focus();
                    }
                    let cmd = if id == trame_chrome::COPY {
                        "copy"
                    } else {
                        "cut"
                    };
                    if let Ok(html_doc) = doc.dyn_into::<web_sys::HtmlDocument>() {
                        let _ = html_doc.exec_command(cmd);
                    }
                }
            } else if id == trame_chrome::PASTE {
                // Paste (sec.5.E): `execCommand("paste")` is blocked by browsers,
                // so read the OS clipboard via the async Clipboard API (allowed in
                // this click gesture). `read()` recovers the private Ambre payload
                // (full styled / formula-reindexed paste, like Ctrl+V) when
                // present, else the plain text; the Logic thread resolves the
                // target from the tracked active cell.
                if let Some(document_id) = current_doc.get_untracked() {
                    let clipboard = web_sys::window().map(|w| w.navigator().clipboard());
                    // `navigator.clipboard` is undefined outside a secure context
                    // (http:// on a LAN IP, etc.): there the button cannot read at
                    // all -- tell the user instead of failing silently.
                    let available = clipboard
                        .as_ref()
                        .map(|c| {
                            let v: &wasm_bindgen::JsValue = c.as_ref();
                            !v.is_undefined() && !v.is_null()
                        })
                        .unwrap_or(false);
                    if let Some(clipboard) = clipboard.filter(|_| available) {
                        let tx = tx.clone();
                        // Call `read()` synchronously, still inside the click's user
                        // gesture: Firefox shows its mandatory "paste" confirmation
                        // right away instead of deferring it to the next click.
                        let read_promise = clipboard.read();
                        wasm_bindgen_futures::spawn_local(paste_from_clipboard(
                            read_promise,
                            clipboard,
                            document_id,
                            tx,
                            toasts,
                        ));
                    } else {
                        toasts.warning(
                            "Clipboard unavailable here (needs HTTPS); use Ctrl+V to paste",
                        );
                    }
                }
            } else if id == trame_chrome::IMPORT_CSV {
                // Interop (`-interop.md` §4.A): open a file picker, read the chosen
                // CSV bytes, and send them to the Logic thread, which creates a new
                // sheet and converts client-side (Zero-Knowledge).
                if let Some(document_id) = current_doc.get_untracked() {
                    open_csv_picker(document_id, tx.clone(), toasts);
                }
            } else {
                dispatch_to_current(id, current_doc, &tx);
            }
        }
    });

    // Formatting toolbar: a control's `StylePatch` -> `SetCellStyle` over the
    // tracked selection (`-ui.md` sec.5bis). The Logic thread resolves the
    // geometry from the selection it keeps via `SetActiveCell`.
    let on_style = Callback::new({
        let tx = to_logic.clone();
        move |patch: ambre_bus::StylePatch| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::SetCellStyle { patch }),
                });
            }
        }
    });

    // Apply the popover's chosen filter to the active column. The Logic thread
    // resolves the sheet/column from the tracked active cell.
    let on_filter_apply = Callback::new({
        let tx = to_logic.clone();
        move |kind: ambre_bus::FilterKind| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::ApplyFilterToActiveColumn { kind }),
                });
            }
        }
    });
    // Find & Replace (sec.5.G): the panel emits these requests; the Logic thread
    // resolves the search origin from the tracked active cell.
    let on_find = Callback::new({
        let tx = to_logic.clone();
        move |q: trame_chrome::FindQuery| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::FindNext {
                        query: q.query,
                        match_case: q.match_case,
                        whole_cell: q.whole_cell,
                    }),
                });
            }
        }
    });
    let on_replace = Callback::new({
        let tx = to_logic.clone();
        move |r: trame_chrome::ReplaceRequest| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::ReplaceOne {
                        query: r.query,
                        replacement: r.replacement,
                        match_case: r.match_case,
                        whole_cell: r.whole_cell,
                        in_formulas: r.in_formulas,
                    }),
                });
            }
        }
    });
    let on_replace_all = Callback::new({
        let tx = to_logic.clone();
        move |r: trame_chrome::ReplaceRequest| {
            if let Some(document_id) = current_doc.get_untracked() {
                let _ = tx.try_send(LogicCommand::App {
                    document_id,
                    command: AppCommand::Trame(TrameCommand::ReplaceAll {
                        query: r.query,
                        replacement: r.replacement,
                        match_case: r.match_case,
                        whole_cell: r.whole_cell,
                        in_formulas: r.in_formulas,
                        all_sheets: r.all_sheets,
                    }),
                });
            }
        }
    });

    let ctx_entries = trame_chrome::context_entries();

    // Per-slot sender clones. Each slot's children is a `Fn` (re-runnable), so it
    // captures its own clone and re-clones inside the handler rather than moving
    // the sender out (which would make the children `FnOnce`).
    let tx_invite = to_logic.clone();
    // The member-row handlers live inside a `<For>` whose item template is a
    // re-runnable `Fn`; a plain `Sender` clone would be moved into it (making it
    // `FnOnce`). A `StoredValue` is `Copy`, so each row reads its own clone.
    let tx_member = StoredValue::new(to_logic.clone());
    let landing_signals = LandingSignals {
        docs,
        username,
        dev_mode,
    };
    let landing_tx = to_logic.clone();

    view! {
        <AppShell>
            <Header slot>
                <span class="title">"Ambre"</span>
                <span class="spacer"></span>
                <span class="ambre-sync">
                    <span class="dot"></span>
                    <span class="label">"Synced"</span>
                </span>
                <button
                    class="icon"
                    title="Toggle theme"
                    aria-label="Toggle colour theme"
                    on:click={
                        let theme_ctl = theme_ctl.clone();
                        move |_| {
                            let next = theme_ctl.toggle();
                            theme_sig.set(next);
                            apply_theme_attr(next);
                            let label = if next == ambre_canvas::Theme::Dark {
                                "Dark theme"
                            } else {
                                "Light theme"
                            };
                            toasts.info(label);
                        }
                    }
                >
                    {move || {
                        let icon = if theme_sig.get() == ambre_canvas::Theme::Dark {
                            icondata::AiMoonFilled
                        } else {
                            icondata::AiSunFilled
                        };
                        view! { <Icon icon=icon /> }
                    }}
                </button>
                <Show when=move || current_doc.get().is_some()>
                    <button
                        class="icon"
                        title="Share"
                        aria-label="Share document"
                        aria-haspopup="dialog"
                        aria-expanded=move || share_open.get().then_some("true")
                        on:click=move |_| share_open.set(true)
                    >
                        <Icon icon=icondata::AiShareAltOutlined />
                    </button>
                </Show>
                <button
                    class="icon"
                    title="About"
                    aria-label="About Ambre"
                    aria-haspopup="dialog"
                    aria-expanded=move || about_open.get().then_some("true")
                    on:click=move |_| about_open.set(true)
                >
                    "?"
                </button>
                <span
                    class="ambre-avatar"
                    title=move || username.get().unwrap_or_else(|| "Signed out".to_owned())
                >
                    {move || initials(username.get().as_deref())}
                </span>
            </Header>
            <Toolbar slot>
                <trame_chrome::FormatBar
                    toggles=toggles
                    style=style_state
                    theme=theme_sig
                    on_command=on_command
                    on_style=on_style
                />
            </Toolbar>
            <FormulaBar slot>
                <trame_chrome::FormulaBar
                    cell_ref=formula_ref
                    content=formula_text
                    on_action=Callback::new(move |action| formula_action.set(Some(action)))
                />
            </FormulaBar>
            <StatusBar slot>
                <trame_chrome::StatusBar stats=sel_stats perf=perf_snap />
            </StatusBar>
        </AppShell>

        <Dialog open=about_open on_open=on_open on_close=on_close class="ambre-about">
            <div class="ambre-about-panel">
                <h2>"Ambre"</h2>
                <p>"Collaborative office suite, 100% Rust, E2EE encrypted."</p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| about_open.set(false)>"Close"</button>
                </div>
            </div>
        </Dialog>

        <Dialog
            open=share_open
            on_open=share_on_open
            on_close=share_on_close
            class="ambre-share"
        >
            <div class="ambre-share-panel">
                <h2>"Share document"</h2>

                // Current members.
                <ul class="ambre-share-members">
                    <For
                        each=move || members.get()
                        key=|m| m.user_id
                        let:member
                    >
                        {
                            let tx_role = tx_member.get_value();
                            let tx_revoke = tx_member.get_value();
                            let uid = member.user_id;
                            let cur_role = member.role;
                            view! {
                                <li>
                                    <span class="ambre-share-member-name">
                                        {member.display_name.clone()}
                                    </span>
                                    <span class="ambre-share-member-email">
                                        {member.email.clone()}
                                    </span>
                                    <select
                                        prop:value=cur_role.as_str()
                                        on:change={
                                            move |ev| {
                                                let Some(document_id) = current_doc.get_untracked() else {
                                                    return;
                                                };
                                                if let Ok(role) = event_target_value(&ev).parse::<Role>() {
                                                    let _ = tx_role.try_send(LogicCommand::Rest {
                                                        id: RequestId::next(),
                                                        cmd: RestCommand::UpdateDocumentMemberRole {
                                                            document_id,
                                                            user_id: uid,
                                                            role,
                                                        },
                                                    });
                                                }
                                            }
                                        }
                                    >
                                        <option value="owner" selected=cur_role == Role::Owner>"Owner"</option>
                                        <option value="editor" selected=cur_role == Role::Editor>"Editor"</option>
                                        <option value="viewer" selected=cur_role == Role::Viewer>"Viewer"</option>
                                    </select>
                                    <button
                                        class="ambre-share-revoke"
                                        title="Revoke access"
                                        aria-label="Revoke access"
                                        on:click={
                                            move |_| {
                                                let Some(document_id) = current_doc.get_untracked() else {
                                                    return;
                                                };
                                                let _ = tx_revoke.try_send(LogicCommand::Rest {
                                                    id: RequestId::next(),
                                                    cmd: RestCommand::RevokeFromDocument {
                                                        document_id,
                                                        user_id: uid,
                                                    },
                                                });
                                            }
                                        }
                                    >
                                        "Revoke"
                                    </button>
                                </li>
                            }
                        }
                    </For>
                </ul>

                // Invite a new collaborator.
                <form
                    class="ambre-share-invite"
                    on:submit={
                        let tx = tx_invite.clone();
                        move |ev| {
                            ev.prevent_default();
                            let Some(document_id) = current_doc.get_untracked() else {
                                return;
                            };
                            let raw = invitee_input.get_untracked();
                            let raw = raw.trim();
                            if raw.is_empty() {
                                return;
                            }
                            // Dev convention: a bare username maps to the
                            // deterministic `<username>@dev.local` email the dev
                            // login assigns (server resolves invitees by email).
                            let invitee_email = if dev_mode.get_untracked() == Some(true)
                                && !raw.contains('@')
                            {
                                format!("{raw}@dev.local")
                            } else {
                                raw.to_owned()
                            };
                            let _ = tx.try_send(LogicCommand::Rest {
                                id: RequestId::next(),
                                cmd: RestCommand::InviteToDocument {
                                    document_id,
                                    invitee_email,
                                    role: invite_role.get_untracked(),
                                },
                            });
                            invitee_input.set(String::new());
                        }
                    }
                >
                    <input
                        r#type="text"
                        class="ambre-share-input"
                        aria-label="Invitee"
                        placeholder=move || {
                            if dev_mode.get() == Some(true) {
                                "username or email"
                            } else {
                                "user@example.com"
                            }
                        }
                        prop:value=move || invitee_input.get()
                        on:input=move |ev| invitee_input.set(event_target_value(&ev))
                    />
                    <select
                        aria-label="Role"
                        on:change=move |ev| {
                            if let Ok(role) = event_target_value(&ev).parse::<Role>() {
                                invite_role.set(role);
                            }
                        }
                    >
                        <option value="editor" selected=true>"Editor"</option>
                        <option value="viewer">"Viewer"</option>
                        <option value="owner">"Owner"</option>
                    </select>
                    <button r#type="submit">"Invite"</button>
                </form>

                <div class="ambre-dialog-actions">
                    <button on:click=move |_| share_open.set(false)>"Close"</button>
                </div>
            </div>
        </Dialog>

        <ContextMenu pos=ctx_menu entries=ctx_entries on_command=on_command />

        // Shared filter popover (sec.5.D), anchored under the active column.
        <trame_chrome::FilterPopover
            state=filter_popover
            values=filter_values
            on_apply=on_filter_apply
            on_command=on_command
        />

        // Find & Replace panel (sec.5.G), pinned top-right.
        <trame_chrome::FindReplace
            state=find_state
            result=find_result
            on_find=on_find
            on_replace=on_replace
            on_replace_all=on_replace_all
        />

        // Live-region toast host (announces transient messages politely).
        <ToastHost toasts=toasts />

        // Landing overlay, shown on `/` over the (frozen, hidden) canvas.
        <Show when=is_landing>
            {
                let landing_tx = landing_tx.clone();
                move || landing::landing_view(landing_signals, landing_tx.clone())
            }
        </Show>
    }
}

/// Dispatch a `CommandId` to the currently routed document, if any. Reads the
/// active document untracked (this runs in an event handler, not a reactive
/// scope) and no-ops on the landing route, where no document is open.
/// Convert a canvas-relative anchor (CSS px, as published by the view for the
/// active cell) into client-space coordinates, by adding the canvas's viewport
/// offset. Falls back to the top-left of the canvas when no anchor is set. Used
/// to position the filter popover under the active column header.
fn client_anchor(anchor: Option<(f64, f64)>) -> (f64, f64) {
    let (ox, oy) = leptos::prelude::document()
        .get_element_by_id("ambre-canvas")
        .map(|c| {
            let r = c.get_bounding_client_rect();
            (r.left(), r.top())
        })
        .unwrap_or((0.0, 0.0));
    let (ax, ay) = anchor.unwrap_or((0.0, 0.0));
    (ox + ax, oy + ay)
}

fn dispatch_to_current(
    id: CommandId,
    current_doc: RwSignal<Option<DocumentId>>,
    to_logic: &flume::Sender<LogicCommand>,
) {
    let Some(document_id) = current_doc.get_untracked() else {
        return;
    };
    if let Err(e) = dispatch_command(id, document_id, to_logic) {
        tracing::warn!("{id:?}: Logic channel full ({e:?})");
    }
}

/// Service the toolbar/menu Paste button by reading the OS clipboard with the
/// async Clipboard API (`-ui.md` sec.5.E). `read()` recovers the private Ambre
/// payload (full styled / formula-reindexed paste) when present, else the plain
/// text; on a browser without `read()` it falls back to `readText()`. The chosen
/// content is handed to the Logic thread, which resolves the target from the
/// tracked active cell. Best-effort: a clipboard the browser will not surface
/// (permission denied, unsupported) is logged, not fatal -- Ctrl+V still works.
/// Open a transient `<input type="file">` picker for a CSV import
/// (`-interop.md` §4.A). On selection it reads the file bytes off the main
/// thread (`File::array_buffer`) and sends a [`TrameCommand::ImportCsv`] to the
/// Logic thread, which converts them into a new sheet client-side. The picker
/// element is never attached to the DOM; the change closure is leaked because the
/// picker is one-shot and lives only until the user picks a file.
fn open_csv_picker(document_id: DocumentId, tx: flume::Sender<LogicCommand>, toasts: Toasts) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(input) = document
        .create_element("input")
        .ok()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return;
    };
    input.set_type("file");
    let _ = input.set_attribute("accept", ".csv,text/csv");

    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        let Some(file) = picker.files().and_then(|fs| fs.get(0)) else {
            return;
        };
        let tx = tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                Ok(buf) => {
                    let data = js_sys::Uint8Array::new(&buf).to_vec();
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command: AppCommand::Trame(TrameCommand::ImportCsv {
                            data,
                            delimiter: None,
                            has_header: false,
                            target_sheet_id: None,
                            start_row: 0,
                            start_col: 0,
                        }),
                    });
                }
                Err(_) => toasts.warning("Could not read the CSV file"),
            }
        });
    });
    input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
    onchange.forget();
    input.click();
}

async fn paste_from_clipboard(
    read_promise: js_sys::Promise,
    clipboard: web_sys::Clipboard,
    document_id: DocumentId,
    tx: flume::Sender<LogicCommand>,
    toasts: Toasts,
) {
    let send = |cmd: TrameCommand| {
        let _ = tx.try_send(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(cmd),
        });
    };

    // Structured read: rich Ambre type first, then plain text. `read_promise` was
    // started synchronously in the click handler so the user gesture is preserved.
    if let Ok(val) = wasm_bindgen_futures::JsFuture::from(read_promise).await
        && let Ok(items) = val.dyn_into::<js_sys::Array>()
    {
        for item in items.iter() {
            let Ok(item) = item.dyn_into::<web_sys::ClipboardItem>() else {
                continue;
            };
            let types = item.types();
            let has = |t: &str| types.iter().any(|v| v.as_string().as_deref() == Some(t));
            if has(host::AMBRE_CLIPBOARD_WEB_TYPE)
                && let Some(payload) = read_clipboard_item(&item, host::AMBRE_CLIPBOARD_WEB_TYPE)
                    .await
                    .and_then(|b64| host::base64_decode(&b64))
                    .and_then(|bytes| {
                        postcard::from_bytes::<ambre_bus::ClipboardPayload>(&bytes).ok()
                    })
            {
                send(TrameCommand::PasteClipboardAtActive { payload });
                return;
            }
            if has("text/plain")
                && let Some(text) = read_clipboard_item(&item, "text/plain").await
            {
                send(TrameCommand::PasteText { text });
                return;
            }
        }
    }

    // Fallback for browsers without structured `read()`.
    if let Ok(val) = wasm_bindgen_futures::JsFuture::from(clipboard.read_text()).await
        && let Some(text) = val.as_string().filter(|t| !t.is_empty())
    {
        send(TrameCommand::PasteText { text });
        return;
    }
    tracing::warn!("Trame: clipboard read unavailable for the Paste button (use Ctrl+V)");
    toasts.warning("Could not read the clipboard; use Ctrl+V to paste");
}

/// Read one MIME entry of a [`web_sys::ClipboardItem`] as a string (its blob
/// decoded as UTF-8). `None` on any failure.
async fn read_clipboard_item(item: &web_sys::ClipboardItem, mime: &str) -> Option<String> {
    let blob_val = wasm_bindgen_futures::JsFuture::from(item.get_type(mime))
        .await
        .ok()?;
    let blob = blob_val.dyn_into::<web_sys::Blob>().ok()?;
    let text_val = wasm_bindgen_futures::JsFuture::from(blob.text())
        .await
        .ok()?;
    text_val.as_string()
}

/// Dev-only: resolve the dev login identity from `?user=<name>` in the boot URL.
///
/// The engine lives for the whole session now (SPA), so a single boot-time
/// `DevLogin` re-derives the session keys once and they persist across client
/// navigations -- no `localStorage` round-trip is needed (the old full-page-load
/// workaround). The landing's dev-login field dispatches `DevLogin` directly
/// thereafter. The `https://`->`{url_scheme}://` fix lives in `ambre-sync`.
fn dev_user_from_url() -> Option<String> {
    let window = web_sys::window()?;
    window
        .location()
        .search()
        .ok()
        .and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
        .and_then(|p| p.get("user"))
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
}

/// localStorage key under which the active dev username is remembered, so a
/// refresh re-establishes the dev session without `?user=` in the URL.
const DEV_USER_KEY: &str = "ambre.dev_user";

/// Read the dev username remembered from a prior session, if any.
fn stored_dev_user() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(DEV_USER_KEY)
        .ok()
        .flatten()
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
}

/// Remember `username` as the active dev identity (dev-only; the derived keys are
/// never stored, only the name they are re-derived from).
pub(crate) fn remember_dev_user(username: &str) {
    if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) {
        let _ = storage.set_item(DEV_USER_KEY, username);
    }
}

/// Map a `SyncWarningCode` (the closed `ambre-bus` contract) to a user-facing
/// notice. All are non-fatal and self-correcting, so the wording reassures
/// rather than alarms. Exhaustive by design: a new variant is a compile error
/// here until it gets a message.
fn sync_warning_message(code: SyncWarningCode) -> &'static str {
    match code {
        SyncWarningCode::QuarantineOverflow => {
            "Syncing is catching up - some changes are taking longer to arrive."
        }
        SyncWarningCode::DecryptFailed => "A remote change could not be read and was skipped.",
    }
}

/// URL of the Sync worker's loader. Trunk builds `ambre-sync` as a separate
/// `data-type="worker"` wasm artifact and emits this `importScripts`-based
/// loader (a CLASSIC worker; it calls `wasm_bindgen(...)`, which creates the
/// worker's own shared `WebAssembly.Memory` and runs `#[wasm_bindgen(start)]`).
/// `filehash` is off (Trunk.toml), so the path is stable.
const SYNC_WORKER_URL: &str = "/ambre-sync_loader.js";

/// Fetch the server's public auth config (`GET /api/auth/config`) and return
/// its `dev_mode` flag. Same-origin, unauthenticated; on any failure (network,
/// non-JSON, missing field) it defaults to `false`, so production falls back to
/// the OIDC sign-in affordance.
async fn fetch_dev_mode() -> bool {
    async fn inner() -> Option<bool> {
        let window = web_sys::window()?;
        let req = web_sys::Request::new_with_str("/api/auth/config").ok()?;
        let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&req))
            .await
            .ok()?;
        let resp: web_sys::Response = resp_val.dyn_into().ok()?;
        let json = wasm_bindgen_futures::JsFuture::from(resp.json().ok()?)
            .await
            .ok()?;
        js_sys::Reflect::get(&json, &"dev_mode".into())
            .ok()
            .and_then(|v| v.as_bool())
    }
    inner().await.unwrap_or(false)
}

/// Up to two uppercase initials from a display name, for the header avatar.
/// Empty / missing name falls back to a neutral dash.
fn initials(name: Option<&str>) -> String {
    let Some(name) = name else {
        return "-".to_owned();
    };
    let letters: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if letters.is_empty() {
        "-".to_owned()
    } else {
        letters
    }
}

fn spawn_sync_worker(to_logic: flume::Sender<LogicEvent>) -> web_sys::Worker {
    // Classic worker (the Trunk loader uses `importScripts`); not a module worker.
    let worker = web_sys::Worker::new(SYNC_WORKER_URL).expect("failed to spawn Sync worker");

    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            let bytes = js_sys::Uint8Array::new(&e.data()).to_vec();
            match postcard::from_bytes::<LogicEvent>(&bytes) {
                Ok(event) => {
                    let _ = to_logic.send(event);
                }
                Err(err) => tracing::error!("[Sync->UI] failed to decode LogicEvent: {err}"),
            }
        });
    worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // Leak the closure: it lives for the whole session (the worker never stops).
    onmessage.forget();
    worker
}

/// Encode-free postMessage of raw `bytes` to the Sync worker, transferring the
/// backing `ArrayBuffer` (zero-copy move, no structured clone of the payload).
fn post_to_worker(worker: &web_sys::Worker, bytes: &[u8]) {
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let buffer = array.buffer();
    let transfer = js_sys::Array::of1(&buffer);
    if let Err(e) = worker.post_message_with_transfer(&buffer, &transfer) {
        tracing::warn!("[UI->Sync] postMessage to Sync worker failed: {e:?}");
    }
}

/// Translate a `CommandId` into its `LogicCommand` and send it to the Logic
/// thread. The single place where UI affordances become IPC payloads; every
/// trigger routes through here.
///
/// Returns `Ok(())` when the command was enqueued, or an error when the Logic
/// channel is full (lossless traffic class: formatting commands must not be
/// silently dropped).
fn dispatch_command(
    id: CommandId,
    document_id: DocumentId,
    to_logic: &flume::Sender<LogicCommand>,
) -> Result<(), flume::TrySendError<LogicCommand>> {
    let Some(command) = trame_chrome::command_to_app(id) else {
        tracing::warn!("no LogicCommand mapping for {id:?}");
        return Ok(());
    };
    to_logic.try_send(LogicCommand::App {
        document_id,
        command,
    })
}

/// Inject the generated token stylesheet (`theme.css`) into `<head>` and set the
/// initial `data-theme`. The chrome then reads colours/spacings as CSS custom
/// properties; the canvas reads the matching `Palette` -- one source, `tokens.ron`.
fn inject_theme(theme: ambre_canvas::Theme) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(Some(head)) = document.query_selector("head") {
        if let Ok(style) = document.create_element("style") {
            style.set_id("ambre-theme");
            style.set_text_content(Some(ambre_canvas::theme::THEME_CSS));
            let _ = head.append_child(&style);
        }
    }
    apply_theme_attr(theme);
}

/// Set the document element's `data-theme`, switching the active CSS custom-property
/// set (`:root` light vs `:root[data-theme="dark"]`).
fn apply_theme_attr(theme: ambre_canvas::Theme) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = el.set_attribute("data-theme", theme.data_attr());
    }
}

/// Attach a `contextmenu` listener to the canvas: right-click opens the DOM
/// context menu at the click point (and suppresses the browser menu). Runs in
/// a post-mount effect so the AppShell's `<canvas>` is already in the DOM.
fn install_canvas_context_menu(ctx_menu: RwSignal<Option<(f64, f64)>>) {
    Effect::new(move |_| {
        let Some(canvas) = leptos::prelude::document().get_element_by_id("ambre-canvas") else {
            return;
        };
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
            e.prevent_default();
            ctx_menu.set(Some((e.client_x() as f64, e.client_y() as f64)));
        });
        let _ = canvas.add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref());
        cb.forget();
    });
}

/// Install the global command-keybinding listener (the keymap's DOM half).
/// A `keydown` whose chord resolves in the keymap is owned: default prevented,
/// command dispatched to the routed document. Unmatched keys fall through to the
/// canvas listener (content navigation), so this never steals arrow keys.
fn install_command_keys(
    keymap: Keymap,
    current_doc: RwSignal<Option<DocumentId>>,
    to_logic: flume::Sender<LogicCommand>,
    find_state: RwSignal<Option<trame_chrome::FindReplaceState>>,
) {
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");

    let cb =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            let Some(chord) = ambre_chrome::keybinding_from_event(&event) else {
                return;
            };
            let Some(id) = keymap.resolve(chord) else {
                return;
            };
            event.prevent_default();
            // Find & Replace (sec.5.G) are UI-only intents: open the DOM panel in
            // the matching mode rather than dispatching to the Logic thread.
            if id == trame_chrome::FIND {
                find_state.set(Some(trame_chrome::FindReplaceState {
                    mode: trame_chrome::FindMode::Find,
                }));
                return;
            }
            if id == trame_chrome::REPLACE {
                find_state.set(Some(trame_chrome::FindReplaceState {
                    mode: trame_chrome::FindMode::Replace,
                }));
                return;
            }
            dispatch_to_current(id, current_doc, &to_logic);
        });
    document
        .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
        .expect("add command keydown listener");
    cb.forget();
}
