# Ambre -- local / CI parity recipes.
#
# Every recipe here is the *exact* invocation CI runs (see
# `.github/workflows/ci.yml`), so `just <recipe>` locally reproduces a CI job
# bit-for-bit. This is the single source of truth for test/lint commands;
# the workflow calls these recipes rather than duplicating the command lines.
#
# Tiers (architecture spec section 4 "Test tiering"):
#   native -- pure Rust on the native x86_64 target, zero external dependency
#             (default tier; the spec calls this the "host" tier).
#   db     -- server integration behind the `db` Cargo feature (postgres/redis/minio).
#   wasm   -- `cfg(target_arch = "wasm32")` surfaces via headless Chrome.

# List recipes.
default:
    @just --list

# Host target only. The wasm `-A warnings` rustflag is scoped to
# `[target.wasm32-unknown-unknown]` in `.cargo/config.toml`, so host clippy
# keeps every warning and `-D warnings` makes them fatal.

# Lint (native): fmt + clippy with warnings denied.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

# The server is excluded from `default-members` (native target), so it is
# named explicitly with a second invocation.

# native tier: workspace logic + the server on the native x86_64 target.
test-native:
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
#
# RUSTFLAGS is overridden here to drop the threading flags that
# `.cargo/config.toml` applies to every wasm32 build (+atomics, --shared-memory,
# --import-memory, ...). A shared-memory module needs a SharedArrayBuffer, which
# the browser only exposes on a cross-origin-isolated page (COOP/COEP). The
# `wasm-bindgen-test-runner` server emits no such headers and multi-threaded wasm
# is unsupported by the harness (wasm-bindgen issue #2892), so a threaded test
# module never instantiates: "Loading Wasm module..." hangs, the runner times out
# and kills chromedriver (SIGKILL).
#
# Setting RUSTFLAGS makes cargo ignore the `[target.wasm32-unknown-unknown]`
# rustflags entirely, so the test module is built single-threaded and loads as a
# plain WebAssembly.Memory. These are pure main-thread tests (leptos signals +
# DOM); their behaviour is identical regardless of the memory model, and the
# production build keeps its threaded codegen untouched. `-Z build-std` still
# comes from `.cargo/config.toml`, rebuilding std consistently without atomics.
# `-Awarnings` preserves the wasm warning policy.
test-wasm:
    RUSTFLAGS="-Awarnings" wasm-pack test --headless --chrome crates/ambre-trame

# Host only: the wasm tier is `-Z build-std`-heavy and Chrome-bound, while
# `cargo-llvm-cov` and proptest run far better on host.

# Coverage (native tier): lcov baseline, non-blocking in CI.
cov:
    cargo llvm-cov --workspace --lcov --output-path lcov.info

# Local convenience: bring the db-tier backing services up.
services-up:
    docker compose up -d postgres redis minio

# Local convenience: tear the db-tier backing services down.
services-down:
    docker compose down
