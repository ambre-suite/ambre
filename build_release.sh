#!/usr/bin/env sh
# Production build: 3 standalone WASM binaries + Trunk release bundle.
#
# The release pipeline builds each worker as a lean standalone binary
# (feature "standalone" enables its own #[wasm_bindgen(start)] entry point).
# The UI Thread is built by Trunk as usual.
#
# Output in crates/ambre-app/dist/
#
# Prerequisites: cargo, wasm-bindgen-cli, trunk

set -e

TARGET="wasm32-unknown-unknown"
OUT="target/wasm-workers"

echo "==> Building Logic Worker (release, standalone)..."
cargo build --target "$TARGET" --release --features standalone -p ambre-logic-worker
mkdir -p "$OUT/logic"
wasm-bindgen --target web \
    --out-dir "$OUT/logic" \
    --out-name logic_worker \
    "target/$TARGET/release/ambre_logic_worker.wasm"

echo "==> Building Sync Worker (release, standalone)..."
cargo build --target "$TARGET" --release --features standalone -p ambre-sync-worker
mkdir -p "$OUT/sync"
wasm-bindgen --target web \
    --out-dir "$OUT/sync" \
    --out-name sync_worker \
    "target/$TARGET/release/ambre_sync_worker.wasm"

cp crates/ambre-app/logic_worker_standalone.js "$OUT/logic_worker.js"
cp crates/ambre-app/sync_worker_standalone.js  "$OUT/sync_worker.js"

echo "==> Trunk release build (UI Thread, standalone)..."
trunk build --release --config Trunk.standalone.toml

echo "==> Release bundle in dist/"
