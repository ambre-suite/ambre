// logic_worker.js
// Bootstraps the Logic Worker role from the unified WASM binary (ambre-app).
//
// The Worker is spawned with { name: "logic" } by the UI Thread.
// #[wasm_bindgen(start)] in ambre-app sees that name and calls
// ambre_logic_worker::start() instead of ui_main().

import init from "./ambre-app.js";

init().catch(console.error);
