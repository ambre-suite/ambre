#!/usr/bin/env sh
# Build Logic Worker and Sync Worker as standalone WASM binaries (debug mode).
#
# Output: target/wasm-workers/
#   logic/logic_worker.js        wasm-bindgen JS glue
#   logic/logic_worker_bg.wasm   worker binary
#   sync/sync_worker.js
#   sync/sync_worker_bg.wasm
#   logic_worker.js              bootstrap (imports ./logic/logic_worker.js)
#   sync_worker.js               bootstrap (imports ./sync/sync_worker.js)
#
# Run this before: trunk serve --config Trunk.standalone.toml
# Or use trunk_serve_standalone.sh which runs both steps.
#
# Prerequisites: cargo, wasm-bindgen-cli (cargo install wasm-bindgen-cli)

set -e

TARGET="wasm32-unknown-unknown"
OUT="target/wasm-workers"

echo "==> Building Logic Worker (debug, standalone)..."
cargo build --target "$TARGET" --features standalone -p ambre-logic-worker
mkdir -p "$OUT/logic"
wasm-bindgen --target web \
    --out-dir "$OUT/logic" \
    --out-name logic_worker \
    "target/$TARGET/debug/ambre_logic_worker.wasm"

echo "==> Building Sync Worker (debug, standalone)..."
cargo build --target "$TARGET" --features standalone -p ambre-sync-worker
mkdir -p "$OUT/sync"
wasm-bindgen --target web \
    --out-dir "$OUT/sync" \
    --out-name sync_worker \
    "target/$TARGET/debug/ambre_sync_worker.wasm"

# Copy the standalone JS bootstraps under the expected filenames so that
# index.standalone.html can copy-file them to the Trunk staging area.
cp crates/ambre-app/logic_worker_standalone.js "$OUT/logic_worker.js"
cp crates/ambre-app/sync_worker_standalone.js  "$OUT/sync_worker.js"

echo "==> Worker packages ready in $OUT/"
