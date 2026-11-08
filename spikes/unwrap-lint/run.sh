#!/usr/bin/env bash
# Runs the spike lint on the workspace: the host line, then the wasm lib line.
# Usage: spikes/unwrap-lint/run.sh [host|wasm|all]. Prints the sites on stdout.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$root/rust-toolchain.toml")
sysroot=$(rustc +"$toolchain" --print sysroot)
# Built from outside the repository: the repository config turns on
# `build-std`, which links a second std beside the one rustc_driver links.
(cd / && cargo +"$toolchain" build -q --release \
    --manifest-path "$root/spikes/unwrap-lint/Cargo.toml")
driver="$root/spikes/unwrap-lint/target/release/ambre-unwrap-lint"
out="$root/target/unwrap-lint/sites"
rm -rf "$out"
# The incremental cache keys the lint set: a driver rebuilt with another lint
# set fails on it with an internal compiler error.
export CARGO_INCREMENTAL=0
export RUSTC_WORKSPACE_WRAPPER="$driver" AMBRE_UNWRAP_OUT="$out"
export LD_LIBRARY_PATH="$sysroot/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cd "$root"
# A fresh unit runs no driver: drop the fingerprints of the workspace units so
# each one runs again. A touch of the sources would stale the clippy units too.
target_dir=${LINT_TARGET_DIR:-target/unwrap-lint/build}
rm -rf "$target_dir"/debug/.fingerprint/ambre-* \
    "$target_dir"/wasm32-unknown-unknown/debug/.fingerprint/ambre-*
features=ambre-logic/fixtures,ambre-toile/pdf,ambre-logic/toile-pdf,ambre-server/db,ambre-server/test-helpers,ambre-canvas/tokens,ambre-sync/fuzz,ambre-belt/browser
line=${1:-all}
if [[ $line == fixture ]]; then
    # The fixture holds one site per approximation of the syn report. A diff
    # against the expected sites fails the run.
    tmp=$(mktemp -d)
    "$driver" rustc --edition 2024 --crate-name ambre_fixture --crate-type lib \
        --emit metadata -o "$tmp/fixture.rmeta" spikes/unwrap-lint/fixture/fixture.rs
    diff spikes/unwrap-lint/fixture/expected.tsv "$out"/ambre_fixture-*.tsv
    echo "fixture: the sites match"
    exit 0
fi
if [[ $line == host || $line == all ]]; then
    cargo check -q --workspace --lib --bins --features "$features" \
        --target-dir "$target_dir"
fi
if [[ $line == wasm || $line == all ]]; then
    RUSTFLAGS="-C debuginfo=0" cargo check -q --target wasm32-unknown-unknown --workspace \
        --exclude ambre-server --exclude ambre-sfu --lib --target-dir "$target_dir"
fi
cat "$out"/*.tsv | sort -u
