// ambre-app/src/lib.rs
// ============================================================================
// Launcher -- the cdylib entry point (#[wasm_bindgen(start)]), running on the
// page's main JS thread (the UI thread).
//
// Wires the three threads and the UI-thread command model:
//   - Create the inter-thread `flume` channels; spawn Logic and Sync.
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
    AppCommand, AppEvent, DocumentId, LogicCommand, LogicEvent, SyncCommand, TrameCommand,
    TrameEvent, UiEvent,
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

    // Inter-thread channels.
    //   UI    --LogicCommand--> Logic      Logic --UiEvent--> UI
    //   Logic --SyncCommand---> Sync       Sync  --LogicEvent--> Logic
    //
    // Bounded (backpressure). The UI runs on the page main thread and must
    // never block, so sends use `try_send` and drop on `Full`.
    let (ui_to_logic_tx, ui_to_logic_rx) = flume::bounded::<LogicCommand>(CHANNEL_CAPACITY);
    let (logic_to_ui_tx, logic_to_ui_rx) = flume::bounded::<UiEvent>(CHANNEL_CAPACITY);
    let (logic_to_sync_tx, logic_to_sync_rx) = flume::bounded::<SyncCommand>(CHANNEL_CAPACITY);
    let (sync_to_logic_tx, sync_to_logic_rx) = flume::bounded::<LogicEvent>(CHANNEL_CAPACITY);

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

    // Reactive state created during mount (needs a Leptos owner) and pulled back
    // out here for the dispatcher to drive: the command-state store and the
    // floating-toolbar anchor signal.
    let store_slot: Rc<RefCell<Option<CommandStates>>> = Rc::new(RefCell::new(None));
    let anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>> =
        Rc::new(RefCell::new(None));
    let toasts_slot: Rc<RefCell<Option<Toasts>>> = Rc::new(RefCell::new(None));
    mount_app_shell(
        ui_to_logic_tx.clone(),
        Rc::clone(&store_slot),
        Rc::clone(&anchor_slot),
        Rc::clone(&toasts_slot),
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

    // Keyboard: resolve registered chords and dispatch the matching command.
    install_command_keys(keymap, ui_to_logic_tx.clone());

    // Spawn the Logic thread. Owns its half of every channel from here on.
    wasm_thread::Builder::new()
        .name("logic".to_owned())
        .spawn(move || {
            ambre_logic::run(ui_to_logic_rx, logic_to_ui_tx, logic_to_sync_tx, sync_to_logic_rx);
        })
        .expect("spawn Logic thread");

    // Spawn the Sync thread (non-blocking async loop in WASM).
    let api_base = web_sys::window()
        .and_then(|w| w.location().host().ok())
        .unwrap_or_else(|| "localhost".into());
    wasm_thread::Builder::new()
        .name("sync".to_owned())
        .spawn(move || {
            wasm_bindgen_futures::spawn_local(ambre_sync::run_wasm(
                logic_to_sync_rx,
                sync_to_logic_tx,
                api_base,
            ));
        })
        .expect("spawn Sync thread");

    // The single UI-thread dispatcher, pumped once per frame: drain Logic->UI,
    // mirror command-state events into the chrome store, forward everything to
    // the canvas view. Being the sole writer of the chrome's toggle state is
    // what keeps every trigger in sync.
    let anchor_pub_pump = anchor_pub.clone();
    let mut last_anchor: Option<(f64, f64)> = None;
    let pre_frame = move || {
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
            TrameView::new(ui_to_logic_tx, view_rx, w, h, text_input, anchor_pub, theme_ctl)
        },
        pre_frame,
        raf,
    );
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

/// Translate a `CommandId` into its `LogicCommand` and send it to the Logic
/// thread. The single place where UI affordances become IPC payloads; every
/// trigger routes through here.
fn dispatch_command(id: CommandId, to_logic: &flume::Sender<LogicCommand>) {
    let command = if id == BOLD {
        AppCommand::Trame(TrameCommand::ToggleBold)
    } else if id == ITALIC {
        AppCommand::Trame(TrameCommand::ToggleItalic)
    } else {
        tracing::warn!("no LogicCommand mapping for {id:?}");
        return;
    };
    // Placeholder document_id — will be replaced by the active document
    // selected from the user's document list once the full auth +
    // ListDocuments + OpenDocument flow is wired in the chrome.
    let document_id = DocumentId::new();
    tracing::debug!("dispatch_command without active document (placeholder)");
    let _ = to_logic.try_send(LogicCommand::App {
        document_id,
        command,
    });
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
    store_slot: Rc<RefCell<Option<CommandStates>>>,
    anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>>,
    toasts_slot: Rc<RefCell<Option<Toasts>>>,
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
            move |id: CommandId| dispatch_command(id, &tx)
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
                    <span class="ambre-avatar">"MC"</span>
                </Header>
                <Toolbar slot>
                    // aria-pressed reflects the authoritative state, whatever
                    // triggered it; the handler only invokes the CommandId.
                    <button
                        title="Bold"
                        aria-pressed=move || bold.get().then_some("true")
                        on:click={
                            let tx = to_logic.clone();
                            move |_| dispatch_command(BOLD, &tx)
                        }
                    >
                        "B"
                    </button>
                    <button
                        title="Italic"
                        aria-pressed=move || italic.get().then_some("true")
                        on:click={
                            let tx = to_logic.clone();
                            move |_| dispatch_command(ITALIC, &tx)
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
                            move |_| dispatch_command(BOLD, &tx)
                        }
                    >
                        "B"
                    </button>
                    <button
                        title="Italic"
                        aria-pressed=move || italic.get().then_some("true")
                        on:click={
                            let tx = to_logic_sel.clone();
                            move |_| dispatch_command(ITALIC, &tx)
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
fn install_command_keys(keymap: Keymap, to_logic: flume::Sender<LogicCommand>) {
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
            dispatch_command(id, &to_logic);
        },
    );
    document
        .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
        .expect("add command keydown listener");
    cb.forget();
}
