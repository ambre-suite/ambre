# Ambre -- local / CI parity recipes.
#
# Every recipe here is the *exact* invocation CI runs (see
# `.github/workflows/ci.yml`), so `just <recipe>` locally reproduces a CI job
# bit-for-bit. This is the single source of truth for test/lint commands;
# the workflow calls these recipes rather than duplicating the command lines.
#
# Tiers (architecture spec section 4 "Test tiering"):
#   host  -- pure Rust on x86_64, zero external dependency (default tier).
#   db    -- server integration behind the `db` Cargo feature (postgres/redis/minio).
#   wasm  -- `cfg(target_arch = "wasm32")` surfaces via headless Chrome.

# List recipes.
default:
    @just --list

# Host target only. The wasm `-A warnings` rustflag is scoped to
# `[target.wasm32-unknown-unknown]` in `.cargo/config.toml`, so host clippy
# keeps every warning and `-D warnings` makes them fatal.

# Lint (host): fmt + clippy with warnings denied.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

# The server is excluded from `default-members` (native target), so it is
# named explicitly with a second invocation.

# host tier: workspace logic + the server on the host target.
test-host:
    cargo test --workspace
    cargo test -p ambre-server

# The `db` feature flips the `cfg_attr(not(feature = "db"), ignore = ...)` gate
# off so the otherwise ignored tests run. Bring services up first with
# `just services-up` (CI declares them as `services:` instead).

# db tier: server integration tests (needs postgres/redis/minio).
test-db:
    cargo test -p ambre-server --features db

# `wasm-pack` drives `wasm-bindgen-test-runner`; the pinned nightly + rust-src
# (rust-toolchain.toml) supply `-Z build-std`.

# wasm tier: cfg(wasm32) surfaces under headless Chrome.
test-wasm:
    wasm-pack test --headless --chrome crates/ambre-trame

# Host only: the wasm tier is `-Z build-std`-heavy and Chrome-bound, while
# `cargo-llvm-cov` and proptest run far better on host.

# Coverage (host tier): lcov baseline, non-blocking in CI.
cov:
    cargo llvm-cov --workspace --lcov --output-path lcov.info

# Local convenience: bring the db-tier backing services up.
services-up:
    docker compose up -d postgres redis minio

# Local convenience: tear the db-tier backing services down.
services-down:
    docker compose down
