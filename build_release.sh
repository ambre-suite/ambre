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

WORKERS_DIR="crates/ambre-app/workers"
TARGET="wasm32-unknown-unknown"

echo "==> Building Logic Worker (release, standalone)..."
cargo build --target "$TARGET" --release --features standalone -p ambre-logic-worker
mkdir -p "$WORKERS_DIR/logic"
wasm-bindgen --target web \
    --out-dir "$WORKERS_DIR/logic" \
    --out-name logic_worker \
    "target/$TARGET/release/ambre_logic_worker.wasm"

echo "==> Building Sync Worker (release, standalone)..."
cargo build --target "$TARGET" --release --features standalone -p ambre-sync-worker
mkdir -p "$WORKERS_DIR/sync"
wasm-bindgen --target web \
    --out-dir "$WORKERS_DIR/sync" \
    --out-name sync_worker \
    "target/$TARGET/release/ambre_sync_worker.wasm"

echo "==> Trunk release build (UI Thread)..."
cd crates/ambre-app && trunk build --release

echo "==> Release bundle in crates/ambre-app/dist/"
