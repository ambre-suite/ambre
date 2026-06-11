// ambre-app/src/lib.rs
// ============================================================================
// Launcher -- the cdylib entry point (#[wasm_bindgen(start)]), running on the
// page's main JS thread (the UI thread).
//
// Wires the execution contexts and the UI-thread command model:
//   - UI (this main thread) + Logic (a `wasm_thread` worker) share this Main
//     wasm instance's memory and talk over `flume` channels.
//   - Sync is a SEPARATE wasm instance in its own Web Worker (own memory),
//     spawned here via `web_sys::Worker` and reached over `postMessage`. The
//     main thread relays Logic<->Sync traffic (drain the Logic->Sync flume and
//     postMessage it; decode worker messages into the Sync->Logic flume).
//   - Mount the DOM chrome (ambre-chrome AppShell) + its command-state store.
//   - Install the command keymap (keyboard -> CommandId).
//   - Run the canvas (ambre_canvas::run_with), pumping a single UI-thread
//     DISPATCHER once per frame.
//
// Command model (ambre-spec-ui.md 2.C): every trigger -- toolbar button,
// keyboard shortcut, (later) context menu / palette -- produces the SAME
// `CommandId` -> `LogicCommand`. The authoritative toggle state comes back as
// a `UiEvent` and is fanned out by the dispatcher to BOTH the chrome signals
// and the canvas view. No surface holds its own toggle truth, so the trigger
// is fully decoupled from the effect.
//
// Compiles ONLY on wasm32 targets (depends on web_sys, wgpu, leptos).
// ============================================================================

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::sync_warning::SyncWarningCode;
use ambre_bus::{
    AppCommand, AppEvent, DocumentId, LogicCommand, LogicEvent, RequestId, RestCommand,
    SyncCommand, TrameCommand, TrameEvent, UiEvent,
};
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};
use ambre_chrome::{
    AppShell, CommandStates, ContextMenu, Dialog, Header, Kbd, MenuEntry, StatusBar, ToastHost,
    Toasts, Toolbar,
};
use ambre_trame::ui::TrameView;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

mod landing;
mod route;

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
        // Notify the page that the WASM application has suffered an
        // unrecoverable panic (panic=abort, so no graceful shutdown).
        if let Some(window) = web_sys::window() {
            let msg = info.to_string();
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

    // Handle to the canvas rAF loop, so a fullscreen modal can suspend it.
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

    // Resolve the page route once at boot. `/` is the landing (document list);
    // `/trame/<id>` (etc.) is a specific document. Navigation between the two is a
    // full page load, so each boot commits to one branch (see `route`).
    let route = route::current();

    // Spawn the Logic thread (shares this instance's memory). Owns the UI/Sync
    // halves of its channels; the Sync-bound half (`logic_to_sync_tx`) is relayed
    // to the Sync worker by the per-frame loop, and `sync_to_logic_tx` is fed by
    // the worker's onmessage handler. Needed in BOTH branches (the landing lists
    // documents over the same UI->Logic->Sync path), so it is spawned up front.
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
    if let Some(username) = dev_user_from_url() {
        tracing::info!("Dev login requested for user '{username}'");
        let _ = ui_to_logic_tx.try_send(LogicCommand::Rest {
            id: RequestId::next(),
            cmd: RestCommand::DevLogin { username },
        });
    }

    // Spawn the Sync worker: a SEPARATE wasm instance in its own Web Worker.
    // Its onmessage decodes `LogicEvent`s into `sync_to_logic_tx` (-> Logic).
    // Kept alive for the whole session by being captured in the relay closure.
    let sync_worker = spawn_sync_worker(sync_to_logic_tx);

    // Landing branch: mount the DOM document list and run a relay-only pump (no
    // canvas). The pump forwards Logic->Sync commands and, on `SessionReady`,
    // requests the document list; `DocumentList` fills the page's signal.
    let (doc_type, document_id) = match route {
        route::Route::Landing => {
            let signals_slot: Rc<RefCell<Option<landing::LandingSignals>>> =
                Rc::new(RefCell::new(None));
            landing::mount_landing(Rc::clone(&signals_slot));
            // The landing has no canvas, so the GPU-init path that normally
            // hides the loading overlay (`window.ambreReady`) never runs here.
            // Signal readiness directly once the DOM is mounted.
            signal_ready();
            let signals = signals_slot
                .borrow()
                .clone()
                .expect("landing signals created during mount");

            // Learn whether the server enables dev login, so the signed-out
            // panel can show the right affordance. Unauthenticated, Main-thread
            // fetch -- a pre-session concern, not routed through the Sync worker.
            {
                let dev_mode = signals.dev_mode;
                wasm_bindgen_futures::spawn_local(async move {
                    dev_mode.set(Some(fetch_dev_mode().await));
                });
            }

            let mut list_requested = false;
            run_raf_pump(move || {
                while let Ok(cmd) = logic_to_sync_rx.try_recv() {
                    match postcard::to_allocvec(&cmd) {
                        Ok(bytes) => post_to_worker(&sync_worker, &bytes),
                        Err(e) => tracing::error!("[UI->Sync] encode failed: {e}"),
                    }
                }
                while let Ok(ev) = logic_to_ui_rx.try_recv() {
                    if let UiEvent::Rest { evt, .. } = ev {
                        match evt {
                            ambre_bus::RestEvent::SessionReady { display_name, .. } => {
                                signals.username.set(Some(display_name));
                                if !list_requested {
                                    list_requested = true;
                                    let _ = ui_to_logic_tx.try_send(LogicCommand::Rest {
                                        id: RequestId::next(),
                                        cmd: RestCommand::ListDocuments,
                                    });
                                }
                            }
                            ambre_bus::RestEvent::DocumentList { documents } => {
                                signals.docs.set(
                                    documents
                                        .into_iter()
                                        .map(|d| landing::DocItem {
                                            document_id: d.document_id,
                                            doc_type: d.doc_type,
                                        })
                                        .collect(),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            });
            return;
        }
        route::Route::Doc { doc_type, document_id } => {
            // Open the routed document. The Sync thread treats a server 404 as a
            // local-only doc (lazily created on the first edit); `doc_type` lets
            // it create the right kind.
            let _ = ui_to_logic_tx.try_send(LogicCommand::OpenDocument { document_id, doc_type });
            (doc_type, document_id)
        }
    };
    let _ = doc_type;

    // ---- Document branch: mount the chrome + canvas around `document_id`. ----

    // Reactive state created during mount (needs a Leptos owner) and pulled back
    // out here for the dispatcher to drive: the command-state store and the
    // floating-toolbar anchor signal.
    let store_slot: Rc<RefCell<Option<CommandStates>>> = Rc::new(RefCell::new(None));
    let anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>> =
        Rc::new(RefCell::new(None));
    let toasts_slot: Rc<RefCell<Option<Toasts>>> = Rc::new(RefCell::new(None));
    let user_slot: Rc<RefCell<Option<RwSignal<Option<String>>>>> = Rc::new(RefCell::new(None));
    mount_app_shell(
        ui_to_logic_tx.clone(),
        document_id,
        Rc::clone(&store_slot),
        Rc::clone(&anchor_slot),
        Rc::clone(&toasts_slot),
        Rc::clone(&user_slot),
        theme_ctl.clone(),
        raf.clone(),
    );
    let store = store_slot
        .borrow()
        .clone()
        .expect("CommandStates created during mount");
    let anchor_pos = anchor_slot
        .borrow()
        .expect("anchor signal created during mount");
    let toasts = toasts_slot
        .borrow()
        .expect("Toasts created during mount");
    let username = user_slot
        .borrow()
        .expect("user signal created during mount");

    // Keyboard: resolve registered chords and dispatch the matching command.
    install_command_keys(keymap, document_id, ui_to_logic_tx.clone());

    // Local persistence: open the IndexedDB store on this (Main) thread, where
    // the event loop is live. On open, restore THIS document's snapshot so an
    // edit survives a refresh without any network round-trip. The store is
    // shared with the per-frame relay below, which saves snapshots Logic emits.
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
            if let Ok(Some(snapshot)) =
                store.load_snapshot(*document_id.as_bytes()).recv_async().await
            {
                tracing::info!("Persistence: restoring doc ({} bytes)", snapshot.len());
                let _ = restore_tx.try_send(LogicCommand::RestoreLocalSnapshot {
                    document_id,
                    snapshot,
                });
            }
            *store_slot.borrow_mut() = Some(store);
        });
    }

    // The single UI-thread dispatcher, pumped once per frame: drain Logic->UI,
    // mirror command-state events into the chrome store, forward everything to
    // the canvas view. Being the sole writer of the chrome's toggle state is
    // what keeps every trigger in sync.
    let anchor_pub_pump = anchor_pub.clone();
    let persist_store_frame = Rc::clone(&persist_store);
    let mut last_anchor: Option<(f64, f64)> = None;
    let pre_frame = move || {
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
                // Session bootstrap (dev `?user=` or OIDC): show the user in the header.
                UiEvent::Rest { evt: ambre_bus::RestEvent::SessionReady { display_name, .. }, .. } => {
                    username.set(Some(display_name.clone()));
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
    };

    // Run the UI thread on this (main) thread, driving the Trame view.
    ambre_canvas::run_with(
        "ambre-canvas",
        move |w, h, text_input| {
            TrameView::new(document_id, ui_to_logic_tx, view_rx, w, h, text_input, anchor_pub, theme_ctl)
        },
        pre_frame,
        raf,
    );
}

/// Dev-only: resolve the dev login identity for this page load.
///
/// Prefers an explicit `?user=<name>` query parameter; when present it is also
/// persisted to `localStorage`. When absent (e.g. a `/trame/<id>` document
/// route, reached from the landing via a full-page navigation that drops the
/// query string) it falls back to the persisted identity.
///
/// This matters because every route change is a full page load: each reload
/// spawns a fresh Sync worker with no in-memory session keys, so the worker must
/// re-run `DevLogin` to re-derive them. The session *cookie* survives the
/// navigation, so REST GETs still authenticate -- but without re-login the
/// worker has no `signing_key` and silently drops every outbound update.
///
/// The Sync worker cannot read the page URL (a dedicated worker's `location` is
/// its own script), so the Main thread resolves it here and forwards `DevLogin`.
fn dev_user_from_url() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok().flatten();

    let from_url = window
        .location()
        .search()
        .ok()
        .and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
        .and_then(|p| p.get("user"))
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());

    if let Some(user) = from_url {
        // Remember the identity so the document routes (no query string) can
        // re-authenticate the fresh worker they each spawn.
        if let Some(s) = &storage {
            let _ = s.set_item("ambre_dev_user", &user);
        }
        return Some(user);
    }

    storage
        .and_then(|s| s.get_item("ambre_dev_user").ok().flatten())
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
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

/// Spawn the Sync worker (a separate wasm instance) and wire its inbound stream:
/// each `postMessage` carries a postcard-encoded `LogicEvent`, decoded here and
/// pushed onto `to_logic` (-> Logic thread). The returned `Worker` must be kept
/// alive (it is, by the per-frame relay closure).
/// Drive `tick` once per animation frame, forever. Used by the landing page,
/// which has no canvas (and therefore no `ambre_canvas::run_with` loop) but still
/// needs to pump the Logic<->Sync relay so the document list can load. The
/// rescheduling `Closure` is leaked into the rAF callback chain to keep it (and
/// everything `tick` captures) alive for the page's lifetime.
/// Call the page's `window.ambreReady` hook, which hides the loading overlay.
/// The canvas host calls this after GPU init on the document route; the landing
/// route (no canvas) must call it explicitly once its DOM is mounted.
fn signal_ready() {
    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"ambreReady".into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }
}

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

fn run_raf_pump(mut tick: impl FnMut() + 'static) {
    let holder: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let holder2 = Rc::clone(&holder);
    *holder.borrow_mut() = Some(Closure::new(move || {
        tick();
        if let (Some(window), Some(cb)) = (web_sys::window(), holder2.borrow().as_ref()) {
            let _ = window.request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }));
    if let (Some(window), Some(cb)) = (web_sys::window(), holder.borrow().as_ref()) {
        let _ = window.request_animation_frame(cb.as_ref().unchecked_ref());
    }
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

/// Mount the application shell (ambre-chrome `AppShell`) onto the page body and
/// publish its command-state store via `store_slot` and the floating-toolbar
/// anchor signal via `anchor_slot`.
///
/// The shell owns the `<canvas id="ambre-canvas">` cell that `ambre_canvas`
/// drives. Toolbar surfaces are pure *views* of the registry: they invoke a
/// `CommandId` and read toggle state from the store -- never their own copy.
fn mount_app_shell(
    to_logic: flume::Sender<LogicCommand>,
    document_id: DocumentId,
    store_slot: Rc<RefCell<Option<CommandStates>>>,
    anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>>,
    toasts_slot: Rc<RefCell<Option<Toasts>>>,
    user_slot: Rc<RefCell<Option<RwSignal<Option<String>>>>>,
    theme_ctl: ambre_canvas::ThemeControl,
    raf: ambre_canvas::RafControl,
) {
    leptos::mount::mount_to_body(move || {
        // Single source of truth for command toggle state, chrome side. Created
        // here (inside the reactive owner) and shared with the dispatcher.
        let store = CommandStates::new(&[BOLD, ITALIC]);
        *store_slot.borrow_mut() = Some(store.clone());

        let bold = store.toggled_signal(BOLD).expect("bold registered");
        let italic = store.toggled_signal(ITALIC).expect("italic registered");

        // Selection-anchor signal: canvas-relative CSS px published by the view
        // (via the dispatcher). The floating toolbar transform-syncs onto it.
        let anchor_pos = RwSignal::new(None::<(f64, f64)>);
        *anchor_slot.borrow_mut() = Some(anchor_pos);

        // Transient notifications in an ARIA live region (announced to screen
        // readers without stealing focus). Demonstrated on the theme toggle.
        let toasts = Toasts::new();
        // Published back to the dispatcher so inbound UiEvents (e.g. SyncWarning)
        // can raise toasts. Same slot pattern as the command-state store.
        *toasts_slot.borrow_mut() = Some(toasts);

        // Current user, filled by the dispatcher when `SessionReady` arrives.
        // Drives the header avatar (initials) and its tooltip (display name).
        let username = RwSignal::new(None::<String>);
        *user_slot.borrow_mut() = Some(username);

        // Theme toggle: flips `theme_ctl` (read by the canvas) and the document's
        // `data-theme` (drives the CSS custom properties). `theme_sig` only
        // drives the button's own glyph.
        let theme_sig = RwSignal::new(theme_ctl.get());

        // About modal: opening it suspends the canvas rAF (frozen behind the
        // blurred backdrop), closing it resumes.
        let about_open = RwSignal::new(false);
        let on_open = Callback::new({
            let raf = raf.clone();
            move |()| raf.pause()
        });
        let on_close = Callback::new({
            let raf = raf.clone();
            move |()| raf.resume()
        });

        // Canvas right-click -> context menu. The launcher owns the listener
        // (the canvas is created by the AppShell); it sets `ctx_menu` to the
        // viewport coords, which the ContextMenu reads (Canvas -> DOM signal).
        let ctx_menu = RwSignal::new(None::<(f64, f64)>);
        install_canvas_context_menu(ctx_menu);
        let ctx_on_command = Callback::new({
            let tx = to_logic.clone();
            move |id: CommandId| {
                if let Err(e) = dispatch_command(id, document_id, &tx) {
                    tracing::warn!("Context menu: Logic channel full ({e:?})");
                }
            }
        });
        let ctx_entries = vec![
            MenuEntry::new(BOLD, "Bold"),
            MenuEntry::new(ITALIC, "Italic"),
        ];

        // The floating toolbar is a separate view root, so it needs its own
        // sender: the AppShell slot's reactive children closure moves `to_logic`.
        let to_logic_sel = to_logic.clone();

        // Floating selection toolbar position. `anchor_pos` is canvas-relative
        // CSS px; the bar is `position: fixed`, so add the canvas's viewport
        // offset (its bounding rect) to land in viewport space. Reading the rect
        // here -- only when the anchor changes -- keeps it glued through pans
        // without per-frame DOM reads in the renderer.
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
                    // aria-pressed reflects the authoritative state, whatever
                    // triggered it; the handler only invokes the CommandId.
                    <button
                        title="Bold"
                        aria-pressed=move || bold.get().then_some("true")
                        on:click={
                            let tx = to_logic.clone();
                            move |_| {
                                if let Err(e) = dispatch_command(BOLD, document_id, &tx) {
                                    tracing::warn!("Bold button: Logic channel full ({e:?})");
                                }
                            }
                        }
                    >
                        "B"
                    </button>
                    <button
                        title="Italic"
                        aria-pressed=move || italic.get().then_some("true")
                        on:click={
                            let tx = to_logic.clone();
                            move |_| {
                                if let Err(e) = dispatch_command(ITALIC, document_id, &tx) {
                                    tracing::warn!("Italic button: Logic channel full ({e:?})");
                                }
                            }
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
                    <p>
                        "Collaborative office suite, 100% Rust, E2EE encrypted."
                    </p>
                    <div class="ambre-dialog-actions">
                        <button on:click=move |_| about_open.set(false)>"Close"</button>
                    </div>
                </div>
            </Dialog>

            <ContextMenu pos=ctx_menu entries=ctx_entries on_command=ctx_on_command />

            // Floating selection toolbar, transform-synced onto the active cell.
            // Same command model as the top toolbar: invoke a CommandId, read
            // toggle state from the store. The outer anchor carries the dynamic
            // translate; the inner pill lifts itself above the cell (CSS).
            <div class="ambre-selbar-anchor" style=selbar_style>
                <div class="ambre-selbar">
                    <button
                        title="Bold"
                        aria-pressed=move || bold.get().then_some("true")
                        on:click={
                            let tx = to_logic_sel.clone();
                            move |_| {
                                if let Err(e) = dispatch_command(BOLD, document_id, &tx) {
                                    tracing::warn!("Floating Bold: Logic channel full ({e:?})");
                                }
                            }
                        }
                    >
                        "B"
                    </button>
                    <button
                        title="Italic"
                        aria-pressed=move || italic.get().then_some("true")
                        on:click={
                            let tx = to_logic_sel.clone();
                            move |_| {
                                if let Err(e) = dispatch_command(ITALIC, document_id, &tx) {
                                    tracing::warn!("Floating Italic: Logic channel full ({e:?})");
                                }
                            }
                        }
                    >
                        <em>"I"</em>
                    </button>
                </div>
            </div>

            // Live-region toast host (announces transient messages politely).
            <ToastHost toasts=toasts />
        }
    });
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
/// command dispatched. Unmatched keys fall through to the canvas listener
/// (content navigation), so this never steals arrow keys or typing.
fn install_command_keys(keymap: Keymap, document_id: DocumentId, to_logic: flume::Sender<LogicCommand>) {
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
            if let Err(e) = dispatch_command(id, document_id, &to_logic) {
                tracing::warn!("Keybinding {id:?}: Logic channel full ({e:?})");
            }
        },
    );
    document
        .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
        .expect("add command keydown listener");
    cb.forget();
}
