// Bootstraps the Sync Worker from its own standalone WASM binary.
// Used in split builds; see Trunk.standalone.toml.
import init from "./sync/sync_worker.js";

init().catch(console.error);
