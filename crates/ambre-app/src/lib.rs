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
    Role, SyncCommand, TrameCommand, TrameEvent, UiEvent,
};
use ambre_canvas::DocSwitcher;
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};
use ambre_chrome::{
    AppShell, CommandStates, ContextMenu, Dialog, Header, Kbd, MenuEntry, StatusBar, ToastHost,
    Toasts, Toolbar,
};
use ambre_trame::ui::TrameView;
use leptos::prelude::*;
use leptos_icons::Icon;
use leptos_router::components::Router;
use leptos_router::hooks::use_location;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

mod landing;
mod route;

use landing::{DocItem, LandingSignals};

/// The Trame "Bold"/"Italic" commands. UI affordance ids (ambre-commands),
/// distinct from the `TrameCommand` IPC payloads they dispatch to.
const BOLD: CommandId = CommandId("trame.format.bold");
const ITALIC: CommandId = CommandId("trame.format.italic");

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
    /// Bold/Italic toggle signals (the store's reactive view, chrome side).
    bold: RwSignal<bool>,
    italic: RwSignal<bool>,
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
}

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

    // Command keymap (the keyboard view of the registry).
    let mut keymap = Keymap::new();
    keymap.bind(KeyBinding::ctrl('b'), BOLD);
    keymap.bind(KeyBinding::ctrl('i'), ITALIC);

    // Handle to the canvas rAF loop, so a fullscreen modal -- or the landing
    // overlay covering the canvas -- can freeze it.
    let raf = ambre_canvas::RafControl::new();

    // Canvas->DOM selection anchor: the view publishes the active cell's screen
    // position, the dispatcher mirrors it into the chrome's `anchor_pos` signal,
    // and the floating selection toolbar transform-syncs onto it.
    let anchor_pub = ambre_canvas::AnchorPublisher::new();

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
    );

    let store = store_slot.borrow().clone().expect("store created during mount");
    let anchor_pos = anchor_slot.borrow().expect("anchor signal created during mount");
    let toasts = toasts_slot.borrow().expect("Toasts created during mount");
    let username = user_slot.borrow().expect("user signal created during mount");
    let docs = docs_slot.borrow().expect("docs signal created during mount");
    let members = members_slot.borrow().expect("members signal created during mount");
    let dev_mode = devmode_slot.borrow().expect("dev_mode signal created during mount");
    let current_doc = current_doc_slot
        .borrow()
        .expect("current_doc signal created during mount");

    // Learn whether the server enables dev login (drives the signed-out panel).
    // Unauthenticated, Main-thread fetch -- a pre-session concern.
    wasm_bindgen_futures::spawn_local(async move {
        dev_mode.set(Some(fetch_dev_mode().await));
    });

    // Keyboard: resolve registered chords and dispatch to the routed document.
    install_command_keys(keymap, current_doc, ui_to_logic_tx.clone());

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
            let store = match ambre_logic::persistence::PersistenceStore::open().recv_async().await {
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
                if let Ok(Some(snapshot)) =
                    store.load_snapshot(*document_id.as_bytes()).recv_async().await
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
    let persist_store_frame = Rc::clone(&persist_store);
    let mut last_anchor: Option<(f64, f64)> = None;
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
                    let _ =
                        route_tx.try_send(LogicCommand::OpenDocument { document_id, doc_type });
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
            match &ev {
                UiEvent::Viewport { event: AppEvent::Trame(te), .. } => match te {
                    TrameEvent::BoldState { enabled } => store.set_toggled(BOLD, *enabled),
                    TrameEvent::ItalicState { enabled } => store.set_toggled(ITALIC, *enabled),
                    _ => {}
                },
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
                            })
                            .collect(),
                    );
                }
                // Share dialog: the current member list landed -> render it.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::DocumentMembers { members: list, .. },
                    ..
                } => members.set(list.clone()),
                // An invite succeeded: confirm and refresh the member list.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::InviteSent { document_id, display_name, .. },
                    ..
                } => {
                    toasts.success(format!("Invited {display_name}"));
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocumentMembers { document_id: *document_id },
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
                        cmd: RestCommand::ListDocumentMembers { document_id: *document_id },
                    });
                }
                // A role change took effect: refresh the member list.
                UiEvent::Rest {
                    evt: ambre_bus::RestEvent::MemberRoleUpdated { document_id, .. },
                    ..
                } => {
                    let _ = relay_tx.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::ListDocumentMembers { document_id: *document_id },
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
    };

    // Boot the canvas (once). It grabs the `<canvas>` the chrome just mounted,
    // drives the Trame view through the rAF loop, and returns the document-swap
    // handle the route effect uses on navigation.
    let view_tx_logic = ui_to_logic_tx.clone();
    let switcher = ambre_canvas::run_with(
        "ambre-canvas",
        move |w, h, text_input| {
            TrameView::new(
                initial_doc,
                view_tx_logic,
                view_rx,
                w,
                h,
                text_input,
                anchor_pub,
                theme_ctl,
            )
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
) {
    leptos::mount::mount_to_body(move || {
        // The store stays here (in the mount owner, NOT in the view): it is `Rc`
        // and the per-frame relay drives it. The shell sees only its toggle
        // signals (Send), so the `<Router>` subtree stays Send.
        let store = CommandStates::new(&[BOLD, ITALIC]);
        let bold = store.toggled_signal(BOLD).expect("bold registered");
        let italic = store.toggled_signal(ITALIC).expect("italic registered");
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

        // Provide the wiring via context rather than as a prop: `<Router>`'s
        // children must be `Fn` (re-runnable), so the children closure cannot
        // consume a moved `wiring`. `AppRoot` clones it back out of context.
        provide_context(Wiring {
            to_logic,
            raf,
            theme_ctl,
            bold,
            italic,
            anchor_pos,
            toasts,
            username,
            docs,
            members,
            dev_mode,
            current_doc,
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
        bold,
        italic,
        anchor_pos,
        toasts,
        username,
        docs,
        members,
        dev_mode,
        current_doc,
    } = use_context::<Wiring>().expect("Wiring provided by mount_shell");

    // The reactive client route, used only to show/hide the landing overlay.
    let location = use_location();
    let is_landing = move || matches!(route::parse(&location.pathname.get()), route::Route::Landing);

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
    let ctx_on_command = Callback::new({
        let tx = to_logic.clone();
        move |id: CommandId| dispatch_to_current(id, current_doc, &tx)
    });
    let ctx_entries = vec![MenuEntry::new(BOLD, "Bold"), MenuEntry::new(ITALIC, "Italic")];

    // Floating selection toolbar position: `anchor_pos` is canvas-relative CSS
    // px; the bar is `position: fixed`, so add the canvas's viewport offset.
    let selbar_style = move || match anchor_pos.get() {
        Some((x, y)) => {
            let (ox, oy) = leptos::prelude::document()
                .get_element_by_id("ambre-canvas")
                .map(|c| {
                    let r = c.get_bounding_client_rect();
                    (r.left(), r.top())
                })
                .unwrap_or((0.0, 0.0));
            format!("transform: translate({}px, {}px)", ox + x, oy + y)
        }
        None => "display: none".to_owned(),
    };

    // Per-slot sender clones. Each slot's children is a `Fn` (re-runnable), so it
    // captures its own clone and re-clones inside the handler rather than moving
    // the sender out (which would make the children `FnOnce`).
    let tx_toolbar = to_logic.clone();
    let tx_selbar = to_logic.clone();
    let tx_invite = to_logic.clone();
    // The member-row handlers live inside a `<For>` whose item template is a
    // re-runnable `Fn`; a plain `Sender` clone would be moved into it (making it
    // `FnOnce`). A `StoredValue` is `Copy`, so each row reads its own clone.
    let tx_member = StoredValue::new(to_logic.clone());
    let landing_signals = LandingSignals { docs, username, dev_mode };
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
                <button
                    title="Bold"
                    aria-pressed=move || bold.get().then_some("true")
                    on:click={
                        let tx = tx_toolbar.clone();
                        move |_| dispatch_to_current(BOLD, current_doc, &tx)
                    }
                >
                    "B"
                </button>
                <button
                    title="Italic"
                    aria-pressed=move || italic.get().then_some("true")
                    on:click={
                        let tx = tx_toolbar.clone();
                        move |_| dispatch_to_current(ITALIC, current_doc, &tx)
                    }
                >
                    <em>"I"</em>
                </button>
                <span class="sep"></span>
                <Kbd binding=KeyBinding::ctrl('b') />
            </Toolbar>
            <StatusBar slot>
                <span>"Trame"</span>
                <span class="spacer"></span>
                <span>"100 %"</span>
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

        <ContextMenu pos=ctx_menu entries=ctx_entries on_command=ctx_on_command />

        // Floating selection toolbar, transform-synced onto the active cell.
        <div class="ambre-selbar-anchor" style=selbar_style>
            <div class="ambre-selbar">
                <button
                    title="Bold"
                    aria-pressed=move || bold.get().then_some("true")
                    on:click={
                        let tx = tx_selbar.clone();
                        move |_| dispatch_to_current(BOLD, current_doc, &tx)
                    }
                >
                    "B"
                </button>
                <button
                    title="Italic"
                    aria-pressed=move || italic.get().then_some("true")
                    on:click={
                        let tx = tx_selbar.clone();
                        move |_| dispatch_to_current(ITALIC, current_doc, &tx)
                    }
                >
                    <em>"I"</em>
                </button>
            </div>
        </div>

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
    let Some(name) = name else { return "-".to_owned() };
    let letters: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if letters.is_empty() { "-".to_owned() } else { letters }
}

fn spawn_sync_worker(to_logic: flume::Sender<LogicEvent>) -> web_sys::Worker {
    // Classic worker (the Trunk loader uses `importScripts`); not a module worker.
    let worker = web_sys::Worker::new(SYNC_WORKER_URL).expect("failed to spawn Sync worker");

    let onmessage = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
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
    let command = if id == BOLD {
        AppCommand::Trame(TrameCommand::ToggleBold)
    } else if id == ITALIC {
        AppCommand::Trame(TrameCommand::ToggleItalic)
    } else {
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
    let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
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
) {
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");

    let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
        move |event: web_sys::KeyboardEvent| {
            let Some(chord) = ambre_chrome::keybinding_from_event(&event) else {
                return;
            };
            let Some(id) = keymap.resolve(chord) else {
                return;
            };
            event.prevent_default();
            dispatch_to_current(id, current_doc, &to_logic);
        },
    );
    document
        .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
        .expect("add command keydown listener");
    cb.forget();
}
