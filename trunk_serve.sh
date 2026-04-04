#!/usr/bin/env sh
# Start the Trunk dev server.
# Trunk builds the unified WASM binary (ambre-app) which serves all three
# roles: UI Thread, Logic Worker, Sync Worker.
#
# Access the app at http://localhost:8080
#
# Prerequisites: cargo, trunk

set -e

# Run from the workspace root so that Trunk.toml (filehash = false, target =
# crates/ambre-app/index.html) is picked up. Without it, Trunk hashes the JS
# glue filename (ambre-app-<hash>.js), breaking the static imports in
# logic_worker.js and sync_worker.js.
trunk serve
