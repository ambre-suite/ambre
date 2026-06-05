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
// ============================================================================

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::{
    AppCommand, AppEvent, IPC_TEST_DOC_ID, LogicCommand, LogicEvent, SyncCommand, TrameCommand,
    TrameEvent, UiEvent,
};
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};
use ambre_chrome::{
    AppShell, CommandStates, ContextMenu, Dialog, Header, Kbd, MenuEntry, StatusBar, Toolbar,
};
use ambre_trame::ui::TrameView;
use leptos::prelude::*;
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
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

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

    // Reactive state created during mount (needs a Leptos owner) and pulled back
    // out here for the dispatcher to drive: the command-state store and the
    // floating-toolbar anchor signal.
    let store_slot: Rc<RefCell<Option<CommandStates>>> = Rc::new(RefCell::new(None));
    let anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>> =
        Rc::new(RefCell::new(None));
    mount_app_shell(
        ui_to_logic_tx.clone(),
        Rc::clone(&store_slot),
        Rc::clone(&anchor_slot),
        raf.clone(),
    );
    let store = store_slot
        .borrow()
        .clone()
        .expect("CommandStates created during mount");
    let anchor_pos = anchor_slot
        .borrow()
        .expect("anchor signal created during mount");

    // Keyboard: resolve registered chords and dispatch the matching command.
    install_command_keys(keymap, ui_to_logic_tx.clone());

    // Spawn the Logic thread. Owns its half of every channel from here on.
    wasm_thread::Builder::new()
        .name("logic".to_owned())
        .spawn(move || {
            ambre_logic::run(ui_to_logic_rx, logic_to_ui_tx, logic_to_sync_tx, sync_to_logic_rx);
        })
        .expect("spawn Logic thread");

    // Spawn the Sync thread.
    wasm_thread::Builder::new()
        .name("sync".to_owned())
        .spawn(move || {
            ambre_sync::run(logic_to_sync_rx, sync_to_logic_tx);
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
            if let UiEvent::Viewport { event: AppEvent::Trame(te), .. } = &ev {
                match te {
                    TrameEvent::BoldState { enabled } => store.set_toggled(BOLD, *enabled),
                    TrameEvent::ItalicState { enabled } => store.set_toggled(ITALIC, *enabled),
                    _ => {}
                }
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
            TrameView::new(ui_to_logic_tx, view_rx, w, h, text_input, anchor_pub)
        },
        pre_frame,
        raf,
    );
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
    let _ = to_logic.try_send(LogicCommand::App {
        document_id: IPC_TEST_DOC_ID,
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
                        "Synced"
                    </span>
                    <button
                        class="icon"
                        title="About"
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

            <Dialog open=about_open on_open=on_open on_close=on_close>
                <h2>"Ambre"</h2>
                <p>
                    "Collaborative office suite, 100% Rust, E2EE encrypted. "
                    "The canvas is frozen while this modal is open "
                    "(rAF suspended)."
                </p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| about_open.set(false)>"Close"</button>
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
        }
    });
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
