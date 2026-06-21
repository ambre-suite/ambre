#!/usr/bin/env sh
# Start the Trunk dev server.
# Trunk builds the unified WASM binary (ambre-app) which serves all three
# roles: UI Thread, Logic Worker, Sync Worker.
#
# Access the app at http://localhost:8080
#
# Usage:
#   ./trunk_serve.sh             # dev profile -- fast incremental rebuilds
#   ./trunk_serve.sh --release   # release profile -- slow, only for perf work
#
# Default to the dev profile: the release profile (opt-level=3, codegen-units=1,
# linker-plugin-lto) recompiles for tens of seconds on every saved edit, which
# masks the watch-triggered rebuild and feels like "nothing happens". Reserve
# --release for measuring real-world performance.
#
# Prerequisites: cargo, trunk

set -e

# Run from the workspace root so that Trunk.toml (filehash = false, target =
# crates/ambre-app/index.html) is picked up. Without it, Trunk hashes the JS
# glue filename (ambre-app-<hash>.js), breaking the static imports in
# logic_worker.js and sync_worker.js.
trunk serve "$@"
