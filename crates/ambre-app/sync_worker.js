// sync_worker.js
// Bootstraps the Sync Worker role from the ambre-app WASM binary.
//
// The Worker is spawned with { name: "sync" } by the UI Thread.
// #[wasm_bindgen(start)] in ambre-app sees that name and calls
// ambre_sync::start() instead of ui_main().
//
// Will go away in Phase 3 of the multi-threaded WASM migration when
// Web Workers are replaced by wasm_thread::spawn over shared memory.

import init from "./ambre-app.js";

init().catch(console.error);
