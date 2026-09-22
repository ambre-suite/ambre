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

# **One invocation for the whole tier.** Cargo caches a build per feature set, so
# four `cargo test` lines under four sets compile the workspace four times over
# and keep all four: 21 min from a clean tree and a `target/` of 18 GB, past the
# 10 GB the shared CI cache holds, so the job's cache is evicted about as often
# as it is restored. The union of the four sets runs every test they run, from
# one build: 10 min 40 s cold, 4.9 GB on disk.
#
# `--workspace` reaches `ambre-server` too: the package is excluded from
# `default-members` (a native target that would break a wasm32 build), which
# `--workspace` overrides.
#
# `--features ambre-logic/fixtures` reaches the fixture machinery, non-default by
# design (`ambre-spec-fixtures.md` §5: `ambre-logic` ships to wasm, so the loader
# and the driver must not be reachable from a client build). It propagates to
# `ambre-core` / `ambre-trame` / `ambre-toile`, each of which declares the
# forward. `just fixtures-check` then runs on the same tier: the belts verify the
# machinery, and the drift gate verifies the committed corpus.
#
# `--features ambre-toile/pdf` reaches the 8 tests behind that feature
# (`logic::interop::pdf_export`, `logic::state::presentation::pdf_tests`,
# `TOILE-IO-005`). A test that never runs verifies nothing.
#
# `--features ambre-logic/toile-pdf` is the same case one crate up. That feature
# carries the handler that answers `ExportPdf` (`TOILE-IO-029`), and a run
# without it compiles the refusal arm instead.
#
# No feature here is read negatively -- no `cfg(not(feature = ...))` exists in
# the workspace -- so enabling all three changes no test's outcome. The `lint`
# recipe still compiles the workspace with no feature at all, which is what holds
# the other half: that the gated code is genuinely optional.

# native tier: the whole workspace, fixture belts and PDF export included, on
# native x86_64, in one build.
test-native:
    cargo test --workspace --features ambre-logic/fixtures,ambre-toile/pdf,ambre-logic/toile-pdf
    just fixtures-check

# Regenerate `fixtures/current/` in place and drop any artifact whose generator
# is gone (`ambre-spec-fixtures.md` section 3). The fix for a red
# `fixtures-check` is always this recipe, never a manual edit of an artifact:
# the bytes are a generator's output, not a document a person maintains. The run
# also applies the frozen retention cap (`FIXTURE-021`, `FIXTURE-026`).
fixtures:
    cargo run -p ambre-logic --features fixtures --bin ambre-fixtures -- generate

# The drift gate. Regenerates into a temporary directory, diffs against the
# committed goldens, and loads every frozen artifact through the migration path.
# Exits non-zero on any drift; runs in the CI host job.
fixtures-check:
    cargo run -p ambre-logic --features fixtures --bin ambre-fixtures -- check

# Copy one artifact out of `current/` into `frozen/`, deriving the version
# suffix from its manifest. Takes a generator name (`trame/grid_literals`),
# because which lineage keeps migration coverage is a deliberate choice.
fixtures-freeze name:
    cargo run -p ambre-logic --features fixtures --bin ambre-fixtures -- freeze {{name}}

# The scale corpora (`ambre-spec-fixtures.md` section 3). Built on demand and
# git-ignored (`FIXTURE-022`): they are multi-megabyte and reproducible from a
# seed, so git would carry them forever for a convenience the seed already gives.
# `--release` because the build is quadratic in the element count and a debug
# build turns minutes into an hour. `toile-pdf` because the deck bench cannot
# compile without Toile's PDF writer, which is behind that non-default feature.
# An artifact already on disk is kept, so a rerun costs nothing.

# scale corpora: write the git-ignored benchmark documents into `fixtures/bench/`.
fixtures-bench-corpus:
    cargo run --release -p ambre-logic --features fixtures,toile-pdf --bin ambre-fixtures -- bench-corpus

# The benches themselves, corpus first. Split from the recipe above because
# generating is minutes and re-running a bench is not: an author iterating on a
# hot path runs this one repeatedly and pays the corpus once.
#
# No CI job runs it. A figure belongs in the pull request with the machine it
# came from (`FIXTURE-023`), because `criterion` measures wall clock and a
# number from one machine means nothing on another.

# benchmarks over the scale corpora, building the corpus when it is absent.
fixtures-bench: fixtures-bench-corpus
    cargo bench -p ambre-logic --features fixtures,toile-pdf --bench scale

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
# and on an unterminated fence (ambre-spec-template.md section 0, Fences rule).
spec-code:
    python3 scripts/spec_code.py --check

# Ledger of the per-spec contract tables (`ambre-contract-ledger.md`), written
# by the script and by nothing else -- a hand-maintained roll-up is the same
# failure mode as a hand-maintained test list: it is wrong the first time a row
# lands without it.
# Also enforces the rules that keep `unimplemented` honest: a declared status
# value, a Story citation on every unbuilt row, and a cited `fn` that resolves.

# contract ledger: fail when the file on disk is not what the script would write.
contracts:
    python3 scripts/contracts.py

# Rewrite the ledger after a contract row changed.
contracts-write:
    python3 scripts/contracts.py --write

# The corpus parsed once, for a reader that is not a human: one record per
# contract row, and the edges each row asserts. The ledger is a roll-up and drops
# the statement, the sources and the mechanism, so a tool needing a row whole
# parses the corpus again to get it back. This holds it parsed, and resolves
# every edge -- the README pillar of a `Sources` cell included, which is the one
# form no other gate reads.

# contract index and graph: fail when the files on disk are not what the corpus says.
index:
    python3 scripts/contract_index.py

# Rewrite the index and the graph after a contract row changed.
index-write:
    python3 scripts/contract_index.py --write

# How a spec is built, not what it quotes: prose share and line length, both
# capped for every spec, production markers in a feature document, the schema
# document's budgets, and every container built in the code present in the
# schema table.

# spec shape: prose share (words), line length, feature-document markers, schema
# budgets, containers.
spec-shape:
    python3 scripts/spec_shape.py --check

# A contract row is seven columns, and four of them answer a reviewer rather than
# a reader: where the rule comes from, which mechanism holds it, what that leaves
# open, and where it stands. They are why the table is trustworthy, and why one
# row wraps over four screen widths. This prints the same rows reduced to the
# three a reader asks for, from the same parser `contracts.py` uses, so the view
# can never state a rule the ledger does not. It writes nothing: a reading view
# on disk is a second copy of every rule, free to go stale.

# reading view of one spec's contract: the id, the kind, the rule.
spec-read FILE:
    python3 scripts/spec_read.py {{FILE}}

# Building one rule means reading the rule, the design that produced it, the rows
# it must not break and the mechanism that would catch it. Today that is the whole
# document plus every document it cites, and nothing says which paragraph mattered.
# This compiles the closure of one id from the index and the graph, so only the
# documents those two name are opened. At the median it is a third of a full read.
# It writes nothing, for the reason `spec-read` writes nothing.

# everything one contract row needs, and nothing else.
spec-context ID:
    python3 scripts/spec_context.py {{ID}}

# What the closure costs over the whole corpus: the least, the median, the most.
spec-context-cost:
    python3 scripts/spec_context.py --all

# Work starts at a story, and the closure above starts at an id. The join is
# already written: a gap that can close names the Story it lands with, and the
# graph carries that citation as an edge. This takes the name, by its wording or
# by its phase code, and compiles every row landing with it. A block two rows
# share is quoted once and named after. A story landing no row says so: the rules
# it writes do not exist yet. The cost goes to standard error, and says when the
# story is cheaper read whole.

# everything one story of the roadmap needs, one closure per row it lands.
spec-context-story NAME:
    python3 scripts/spec_context.py --story "{{NAME}}"

# The three answers under a contract table are one paragraph to a reader, so the
# cap is 150 words: six sentences under the 25-word cap. What fills them when
# nothing holds them is the document's own history -- which conjunction came
# apart, what the pass added, which row carried which half. That is a commit
# message, and the second metric refuses it.

# the answers under a contract table: how many words they spend, and whether any
# line writes the document's own history.
spec-answers:
    python3 scripts/spec_answers.py --check

# Every breach, with its line, so a red check says what to rewrite.
spec-answers-detail:
    python3 scripts/spec_answers.py

# `ai/ambre-writing-style.md` ended on "No script checks this file [...] It is a
# review rule, and review rules drift." It drifted: a Rust comment reached the
# tree with a metaphor, a passive and a 33-word sentence, and no check saw any of
# it. This holds the half a script can hold: printable ASCII, Latin
# abbreviations, sentence and paragraph length, the banned terms and the empty
# openers. Every one is a hard error, so a new file and a new comment start
# clean. Metaphor, ambiguous pronoun and "why, never what" stay a human read.

# writing style: the mechanical half of `ai/ambre-writing-style.md`.
prose:
    python3 scripts/prose.py --check

# Each application freezes, under its compiled `SCHEMA_VERSION`, the key set and
# the value types a build writes. A belt in the crate holds a build against that
# text, and the belt alone is weak: regenerating the section in place turns it
# green with the version never raised, while every peer on the old version reads
# a document it cannot parse. A section is the record of what one version wrote,
# so this refuses any section the base revision carries whose body moved. Pass
# `--base` to name another revision.

# append-only schema freezes: a published shape section never moves.
schema-freeze:
    python3 scripts/schema_freeze.py --check

# The checks in `scripts/` guard every Rust comment, every document and every
# contract row, and nothing guarded the checks. Three defects shipped in the
# prose one, and the worst produced no symptom: a table row left the reader's
# result, so the hard rules never read it. A check that reads nothing and a
# check that is satisfied look alike from outside, which is what these hold,
# one test file per script.

# what each check in `scripts/` reads, and what it refuses.
test-scripts:
    python3 -m unittest discover -s scripts

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
