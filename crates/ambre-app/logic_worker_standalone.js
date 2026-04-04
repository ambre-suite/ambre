// Bootstraps the Logic Worker from its own standalone WASM binary.
// Used in split builds; see Trunk.standalone.toml.
import init from "./logic/logic_worker.js";

init().catch(console.error);
