// sync_worker.js
// Bootstraps the Sync Worker role from the unified WASM binary (ambre-app).
//
// The Worker is spawned with { name: "sync" } by the UI Thread.
// #[wasm_bindgen(start)] in ambre-app sees that name and calls
// ambre_sync_worker::start() instead of ui_main().

import init from "./ambre-app.js";

init().catch(console.error);
