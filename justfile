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

# The fixture machinery sits behind a non-default `fixtures` feature
# (`ambre-spec-fixtures.md` §5: `ambre-logic` ships to wasm, so the loader and
# driver must not be reachable from a client build). `cargo test --workspace`
# passes no `--features`, so it cannot reach that code at all -- hence the extra
# line. `FIXTURES P1` replaces it with `just fixtures-check`, which adds the
# golden drift gate on top of the same belts.
#
# **One invocation for the fixture belts.**
# `--features ambre-logic/fixtures` propagates to `ambre-core` / `ambre-trame` /
# `ambre-toile` (each declares the forward), so their gated code is compiled and
# type-checked as a dependency here. Naming `ambre-trame` as a package too would
# rebuild its whole test harness under a second feature set for no coverage --
# it owns no `fixtures`-gated test -- and this job already carries the workspace
# build plus the native server stack on one runner disk.
#
# **`ambre-toile` is a separate case: it owns 8 tests behind its `pdf` feature**
# (`logic::interop::pdf_export`, `logic::state::presentation::pdf_tests`), which
# `cargo test --workspace` cannot reach because it enables no optional feature.
# They had never executed in CI. A test that never runs verifies nothing, so the
# feature gets its own line rather than a caveat in `toile-interop §1`
# (`TOILE-IO-005`).

# native tier: workspace logic + the fixture belts + the server, on native x86_64.
test-native:
    cargo test --workspace
    cargo test -p ambre-core -p ambre-logic --features ambre-logic/fixtures
    cargo test -p ambre-toile --features pdf
    cargo test -p ambre-server

# The `db` feature flips the `cfg_attr(not(feature = "db"), ignore = ...)` gate
# off so the otherwise ignored tests run. Bring services up first with
# `just services-up` (CI declares them as `services:` instead).
#
# `test-helpers` is the second half: `redis_ops::lua_tests` is
# `cfg(all(test, feature = "test-helpers"))`, so its six Lua/clock tests --
# including the five that verify `SRV-006`, the strict per-client clock -- did
# not even compile under `--features db` alone. 156 tests were collected where
# 162 exist.

# db tier: server integration tests (needs postgres/redis/minio).
test-db:
    cargo test -p ambre-server --features db,test-helpers

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
#
# `-C debuginfo=0` is critical: the dev profile's DWARF balloons the test module
# to ~350 MB, which headless Chrome cannot compile within the runner's 20 s
# detection window ("Loading Wasm module..." hangs, then SIGKILL). Dropping
# debuginfo shrinks it to ~10 MB and the tests load instantly; stack traces are
# irrelevant for these assertion-only UI tests.
test-wasm:
    RUSTFLAGS="-Awarnings -C debuginfo=0" wasm-pack test --headless --chrome crates/ambre-trame
    RUSTFLAGS="-Awarnings -C debuginfo=0" wasm-pack test --headless --chrome crates/ambre-chrome
    RUSTFLAGS="-Awarnings -C debuginfo=0" wasm-pack test --headless --chrome crates/ambre-toile
    RUSTFLAGS="-Awarnings -C debuginfo=0" wasm-pack test --headless --chrome crates/ambre-logic
    RUSTFLAGS="-Awarnings -C debuginfo=0" wasm-pack test --headless --chrome crates/ambre-app

# Host only: the wasm tier is `-Z build-std`-heavy and Chrome-bound, while
# `cargo-llvm-cov` and proptest run far better on host.

# Coverage (native tier): lcov baseline, non-blocking in CI.
cov:
    cargo llvm-cov --workspace --lcov --output-path lcov.info

# A spec states what and why; the source states how (`ambre-spec-template.md §0`).
# Shapes are the contract and stay -- `struct`, `enum`, trait and public
# signatures. Function bodies are a second, uncompiled copy that drifts: a
# `trame-ui` snippet contradicted its own sentence, and `proptest!` bodies
# described a suite that did not exist. This makes the rule mechanical.

# no implementation in the specs: fails on any function body in a ```rust fence,
# on Rust shapes left in a non-rust fence, on a disallowed fence tag (lua, ...),
# and on an unterminated fence (ambre-spec-template.md sec.0, Fences rule).
spec-code:
    python3 scripts/spec_code.py --check

# Ledger of the per-spec contract tables (`ambre-contract-ledger.md`), written
# by the script and by nothing else -- a hand-maintained roll-up is the same
# failure mode as a hand-maintained test list, and it was already wrong by one.
# Also enforces the rules that keep `unimplemented` honest: a declared status
# value, a Story citation on every unbuilt row, and a cited `fn` that resolves.

# contract ledger: fail when the file on disk is not what the script would write.
contracts:
    python3 scripts/contracts.py

# Rewrite the ledger after a contract row changed.
contracts-write:
    python3 scripts/contracts.py --write

# Rewrite the per-document ceiling on `unverified` rows after a deliberate
# change. It lowers only, so it cannot make a red check green: a rising ceiling
# is shipped code losing its last mechanism. A document that legitimately grows
# loose needs the target below, and a reviewed diff.
contracts-ceilings:
    python3 scripts/contracts.py --ceilings

# A deliberate growth of the coverage debt, recorded rather than silent.
contracts-ceilings-raise:
    python3 scripts/contracts.py --ceilings --raise

# How a spec is built, not what it quotes: prose share (a migrated document is
# capped, every other spec carries a ceiling it may only fall below), production
# markers in a feature document, the schema document's budgets, and every
# container built in the code present in the schema table.

# spec shape: prose share (words), line length, feature-document markers, schema
# budgets, containers.
spec-shape:
    python3 scripts/spec_shape.py --check

# Rewrite the per-spec ceilings, prose words and long lines, after a deliberate
# change. It lowers only, so it cannot make a red check green: a rising ceiling
# is a spec getting worse. A spec that legitimately grows needs
# `python3 scripts/spec_shape.py --update --raise` and a reviewed diff.
spec-shape-update:
    python3 scripts/spec_shape.py --update

# `ai/ambre-writing-style.md` ended on "No script checks this file [...] It is a
# review rule, and review rules drift." It drifted: a Rust comment reached the
# tree with a metaphor, a passive and a 33-word sentence, and no check saw any of
# it. This holds the half a script can hold -- printable ASCII, Latin
# abbreviations, sentence and paragraph length, the passive -- as a per-file
# ceiling that only falls. A file the table does not name has a ceiling of zero,
# so a new file and a new comment start clean. Metaphor, ambiguous pronoun and
# "why, never what" stay a human read.

# writing style: the mechanical half of `ai/ambre-writing-style.md`.
prose:
    python3 scripts/prose.py --check

# Every breach, with its line, so a red check says what to rewrite.
prose-detail:
    python3 scripts/prose.py --verbose

# Rewrite the per-file ceilings after a deliberate change. It lowers only, so it
# cannot make a red check green. A file that legitimately grows needs
# `python3 scripts/prose.py --ceilings --raise` and a reviewed diff.
prose-ceilings:
    python3 scripts/prose.py --ceilings

# The corpus cites a document by its name and, half the time, by a section of it.
# Nothing resolved either, so splitting a spec left every stale citation silent:
# `prose-engine` pointed at no document for as long as three Prose specs have
# existed, and renumbering a spec broke every coordinate aimed at it in silence.
# This reads the name, asks whether the file is there, and asks the file whether
# it has that section.

# citation resolution: every document and section cited exists.
spec-refs:
    python3 scripts/spec_refs.py --check

# What the product does today, one row per feature document. The roadmap holds
# what is left to do, and a feature built by three stories across two epics
# appears nowhere whole in it. The inventory carries no fact of its own, so it is
# generated: this regenerates it in memory and fails when the file on disk
# differs, which is what stops a spec edit from leaving a stale product surface.
# It also reads the rows against the header, and fails on a document that claims
# `implemented` beside a row that is not built.

# feature inventory: generated from the feature documents, and never stale.
inventory:
    python3 scripts/feature_inventory.py --check

# Rewrite each application's inventory after a feature document changed.
inventory-write:
    python3 scripts/feature_inventory.py --write

# Supply chain + capability wall. `deny.toml` sets `[graph] all-features = true`,
# so optional dependencies (the `fixtures` feature's `ambre-crypto` edge) are part
# of the graph being checked rather than invisible to it. No network build: this
# reads `Cargo.lock` and the advisory database only.

# cargo-deny: advisories, licences, sources, and the ZK capability bans.
deny:
    cargo deny check

# Local convenience: bring the db-tier backing services up.
services-up:
    docker compose up -d postgres redis minio

# Local convenience: tear the db-tier backing services down.
services-down:
    docker compose down
