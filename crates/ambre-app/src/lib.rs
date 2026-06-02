// ambre-app/src/lib.rs
// ============================================================================
// Launcher — the cdylib entry point (#[wasm_bindgen(start)]), running on the
// page's main JS thread.
//
// Its entire job is to wire the three threads together:
//   - Create the four inter-thread `flume` channels.
//   - Spawn the Logic and Sync threads (ambre_logic::run / ambre_sync::run).
//   - Run the UI thread (ambre_ui::run) on this thread, injecting the Trame
//     view as the application's DocView.
//
// All UI behaviour lives in ambre_ui::run (generic driver) and TrameView
// (the DocView impl). This file knows nothing about rendering or spreadsheets
// beyond choosing which view to instantiate.
// ============================================================================

use wasm_bindgen::prelude::*;

use ambre_bus::{LogicEnvelope, LogicEvent, SyncCommand, UiEnvelope};
use ambre_trame::ui::TrameView;

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
    //   UI    --UiEnvelope--->  Logic
    //   Logic --LogicEnvelope-> UI
    //   Logic --SyncCommand--->  Sync
    //   Sync  --LogicEvent---->  Logic
    let (ui_to_logic_tx, ui_to_logic_rx) = flume::unbounded::<UiEnvelope>();
    let (logic_to_ui_tx, logic_to_ui_rx) = flume::unbounded::<LogicEnvelope>();
    let (logic_to_sync_tx, logic_to_sync_rx) = flume::unbounded::<SyncCommand>();
    let (sync_to_logic_tx, sync_to_logic_rx) = flume::unbounded::<LogicEvent>();

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

    // Run the UI thread on this (main) thread, driving the Trame view.
    ambre_ui::run("ambre-canvas", move |w, h| {
        TrameView::new(ui_to_logic_tx, logic_to_ui_rx, w, h)
    });
}
