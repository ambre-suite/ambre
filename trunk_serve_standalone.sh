#!/usr/bin/env sh
# Start Trunk dev server in standalone (split) mode.
#
# Builds Logic and Sync workers as independent WASM binaries, then starts
# Trunk using index.standalone.html (no "unified" feature in ambre-app).
#
# Access the app at http://localhost:8080
#
# Prerequisites: cargo, wasm-bindgen-cli, trunk

set -e

sh build_workers_dev.sh
trunk serve --config Trunk.standalone.toml
