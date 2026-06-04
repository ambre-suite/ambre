// ambre-app/src/lib.rs
// ============================================================================
// Launcher — the cdylib entry point (#[wasm_bindgen(start)]), running on the
// page's main JS thread.
//
// Its entire job is to wire the three threads together:
//   - Create the four inter-thread `flume` channels.
//   - Spawn the Logic and Sync threads (ambre_logic::run / ambre_sync::run).
//   - Run the UI thread (ambre_canvas::run) on this thread, injecting the Trame
//     view as the application's DocView.
//
// All UI behaviour lives in ambre_canvas::run (generic driver) and TrameView
// (the DocView impl). This file knows nothing about rendering or spreadsheets
// beyond choosing which view to instantiate.
// ============================================================================

use ambre_bus::{LogicCommand, LogicEvent, SyncCommand, UiEvent};
use ambre_trame::ui::TrameView;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;

/// Capacity of every inter-thread channel.
///
/// Deliberately generous: P3 traffic is request/response and never approaches
/// it, and the hot viewport path is coalesced to ≤1 send/frame on the UI side
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

    // Mount the DOM chrome (ambre-chrome AppShell) onto the page body. This
    // creates the full-viewport grid AND the `<canvas>` cell that the UI thread
    // attaches wgpu to below, so it must run before `ambre_canvas::run`.
    mount_app_shell();

    // Inter-thread channels.
    //   UI    --LogicCommand--->  Logic
    //   Logic --UiEvent-> UI
    //   Logic --SyncCommand--->  Sync
    //   Sync  --LogicEvent---->  Logic
    //
    // Bounded (backpressure). Worker-thread senders (Logic, Sync) may block on a
    // full channel — that IS the backpressure. The UI runs on the page's main
    // thread and must never block, so `TrameView` sends with `try_send` and
    // drops on `Full` (the next viewport supersedes a dropped one via `version`).
    let (ui_to_logic_tx, ui_to_logic_rx) = flume::bounded::<LogicCommand>(CHANNEL_CAPACITY);
    let (logic_to_ui_tx, logic_to_ui_rx) = flume::bounded::<UiEvent>(CHANNEL_CAPACITY);
    let (logic_to_sync_tx, logic_to_sync_rx) = flume::bounded::<SyncCommand>(CHANNEL_CAPACITY);
    let (sync_to_logic_tx, sync_to_logic_rx) = flume::bounded::<LogicEvent>(CHANNEL_CAPACITY);

    // Spawn the Logic thread. Owns its half of every channel from here on.
    wasm_thread::Builder::new()
        .name("logic".to_owned())
        .spawn(move || {
            ambre_logic::run(
                ui_to_logic_rx,
                logic_to_ui_tx,
                logic_to_sync_tx,
                sync_to_logic_rx,
            );
        })
        .expect("spawn Logic thread");

    // Spawn the Sync thread.
    wasm_thread::Builder::new()
        .name("sync".to_owned())
        .spawn(move || {
            ambre_sync::run(logic_to_sync_rx, sync_to_logic_tx);
        })
        .expect("spawn Sync thread");

    // Run the UI thread on this (main) thread, driving the Trame view.
    ambre_canvas::run("ambre-canvas", move |w, h| {
        TrameView::new(ui_to_logic_tx, logic_to_ui_rx, w, h)
    });
}

/// Mount the application shell (ambre-chrome `AppShell`) onto the page body.
///
/// The shell is app-agnostic; this is where the Trame app fills its chrome
/// regions. Header/toolbar/status content here is still static placeholder --
/// the toolbar buttons become live registry commands at the end-to-end wiring
/// step. The shell owns the `<canvas id="ambre-canvas">` cell that
/// `ambre_canvas::run` then drives.
fn mount_app_shell() {
    use ambre_chrome::commands::KeyBinding;
    use ambre_chrome::{AppShell, Header, Kbd, StatusBar, Toolbar};

    leptos::mount::mount_to_body(|| {
        view! {
            <AppShell>
                <Header slot>
                    <span class="title">"Ambre"</span>
                    <span class="spacer"></span>
                    <span class="ambre-sync">
                        <span class="dot"></span>
                        "Synchronise"
                    </span>
                    <span class="ambre-avatar">"MC"</span>
                </Header>
                <Toolbar slot>
                    <button title="Gras">"B"</button>
                    <button title="Italique"><em>"I"</em></button>
                    <span class="sep"></span>
                    <Kbd binding=KeyBinding::ctrl('b') />
                </Toolbar>
                <StatusBar slot>
                    <span>"Trame"</span>
                    <span class="spacer"></span>
                    <span>"100 %"</span>
                </StatusBar>
            </AppShell>
        }
    });
}
