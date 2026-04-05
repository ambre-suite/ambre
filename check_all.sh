#!/usr/bin/env sh
# Dual-target cargo clippy for rust-analyzer check.overrideCommand.
# WASM crates use wasm32-unknown-unknown, server uses native host target.
# Both emit newline-delimited JSON consumed by rust-analyzer.
cargo clippy \
    --target wasm32-unknown-unknown \
    -p ambre-app \
    -p ambre-logic-worker \
    -p ambre-sync-worker \
    -p ambre-bus \
    -p ambre-trame \
    -p ambre-draw \
    -p ambre-core \
    -p ambre-crypto \
    -p ambre-protocol \
    -p ambre-sync \
    -p ambre-prose \
    -p ambre-toile \
    -p ambre-murmure \
    --message-format=json 2>&1
# For the native server check, filter out registry dep messages:
# RA's project model is built with wasm32 target, so native-only transitive
# deps (mio, parking, etc.) are unknown to it. We only forward messages from
# workspace crates (path+ package IDs) to avoid "unknown package" log spam.
cargo clippy \
    -p ambre-server \
    --message-format=json 2>&1 \
    | grep -v '"package_id":"registry+'
