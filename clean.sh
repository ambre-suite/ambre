#!/usr/bin/env sh
# Remove all build artifacts.

set -e

rm -rf crates/ambre-app/workers/logic crates/ambre-app/workers/sync
rm -rf crates/ambre-app/dist
cargo clean
