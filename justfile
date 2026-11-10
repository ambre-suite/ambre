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
#   db     -- server integration behind the `db` Cargo feature (postgres/redis/garage).
#   wasm   -- `cfg(target_arch = "wasm32")` surfaces via headless Chrome.

# List recipes.
default:
    @just --list

# Host target only. The wasm `-A warnings` rustflag is scoped to
# `[target.wasm32-unknown-unknown]` in `.cargo/config.toml`, so host clippy
# keeps every warning and `-D warnings` makes them fatal.
#
# Every non-default feature is on, so the code behind one is linted too: the
# fixture machinery, the PDF export and the relay's database half. A panic lint
# that never sees a file holds nothing there (`ARCH-028`). `just primitives`
# refuses a feature a manifest declares that no lint line turns on, unless
# `specs/1-Foundations/ambre-features.tsv` excludes it with its reason
# (`ARCH-033`).

# Lint (native): fmt + clippy with warnings denied.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --features ambre-logic/fixtures,ambre-toile/pdf,ambre-logic/toile-pdf,ambre-pptx/codec,ambre-server/db,ambre-server/test-helpers,ambre-canvas/tokens,ambre-sync/fuzz,ambre-belt/browser -- -D warnings
    cargo clippy -p ambre-belt --all-targets --features ambre-belt/firefox -- -D warnings

# Lint (wasm): the code only the browser compiles -- the application chrome, the
# canvas host, the workers -- which host clippy never reads. `RUSTFLAGS` drops
# the threading flags of `.cargo/config.toml`, as `test-wasm` does, and with
# them the `-A warnings` there. `-D warnings` makes every warning fatal, so a
# warning on wasm-only code fails here as one on the host fails `lint`. The
# Prose and Murmure unit tests are host-only, so their wasm test files are
# linted alone.
lint-wasm:
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown --workspace --exclude ambre-server --exclude ambre-sfu --lib
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-trame -p ambre-toile --tests
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-prose --test chrome_panel --test chrome_ui
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-murmure --test chrome_relay
    # Each converter module as `scripts/converter_build.py` builds it: one
    # family's engine and the exports, with no loader. No line above turns an
    # engine on for wasm, so these read the exports (`ARCH-079`), and each
    # family alone, as its module compiles it.
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-convert --features tables --lib
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-convert --features pdf --lib
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-convert --features markup --lib
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-convert --features images --lib
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-convert --features ooxml --lib
    # The PowerPoint codec, which the converter module will link: no line above
    # turns its `codec` feature on for wasm.
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-pptx --features codec --lib
    # `test-wasm` builds the Logic, app and chrome test targets, and no other
    # wasm line compiles them, so these three lines lint them. A broken wasm
    # test build, or a warning in one, fails here.
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-logic --tests
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-app --tests
    RUSTFLAGS="-D warnings -C debuginfo=0" cargo clippy --target wasm32-unknown-unknown -p ambre-chrome --tests

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
# `--features ambre-toile/pdf` reaches the tests behind that feature
# (`logic::interop::pdf_export`, `logic::state::presentation::pdf_tests`,
# `TOILE-IO-005`). A test that never runs verifies nothing.
#
# `--features ambre-logic/toile-pdf` is the same case one crate up. That feature
# carries the handler that answers `ExportPdf` (`TOILE-IO-029`), and a run
# without it compiles the refusal arm instead.
#
# `--features ambre-pptx/codec` reaches the PowerPoint writer and its tests
# (`toile-pptx-export`). No crate turns the codec on until the converter
# module links it.
#
# `--features ambre-canvas/tokens` reaches the generator and its tests, and
# `just tokens-check` then holds the committed theme files to `tokens.ron`.
#
# No feature here is read negatively -- no `cfg(not(feature = ...))` exists in
# the workspace -- so enabling all four changes no test's outcome. The other
# half is that the gated code is genuinely optional. No recipe compiles the host
# workspace with no feature, so `lint-wasm` holds that half. Its lines compile
# the client crates without `fixtures` or `tokens`. Its Logic and Trame lines
# compile `ambre-logic` and `ambre-toile` without the PDF features, which
# `ambre-app` turns on. This tier compiles `ambre-server` without `db`.

# native tier: the whole workspace, fixture belts and PDF export included, on
# native x86_64, in one build.
test-native:
    cargo test --workspace --features ambre-logic/fixtures,ambre-toile/pdf,ambre-logic/toile-pdf,ambre-pptx/codec,ambre-canvas/tokens
    just fixtures-check
    just tokens-check
    just locale-table-check

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

# The design tokens (`ambre-spec-design-tokens.md`). `tokens.ron` is the one
# source, and the generator writes the committed `src/theme_gen.rs` and
# `src/theme.css` of `ambre-canvas`. They are committed so that every gate reads
# them, and the check below holds them to the source. The fix for a red check
# is always the rewrite, never a hand edit. `ambre-canvas/tokens` is on in the
# test-native build, so the generator's tests run there. The check builds the
# binary under the feature set of `ambre-canvas` alone. There `ambre-core` and
# `ambre-draw` lose a feature, so cargo compiles them and each crate above them
# again: the check reuses none of the Ambre crates of the test build.

# Rewrite the generated theme module and stylesheet from `tokens.ron`.
tokens-write:
    cargo run -q -p ambre-canvas --features tokens --bin ambre-tokens -- --write

# token drift: fail when a generated theme file is not what `tokens.ron` gives.
tokens-check:
    cargo run -q -p ambre-canvas --features tokens --bin ambre-tokens

# The number and date forms of each locale (`ambre-spec-locale.md`), from a
# pinned CLDR. `locale-table-fetch` is the one step that reads the network: it
# refreshes `scripts/data/cldr-locale-subset.json` from the npm registry, each
# package checked against its pinned integrity. `locale-table` writes the Rust
# table from that subset, and `locale-table-check` fails, offline, on a table
# the subset does not give. No value of the table is typed by hand.
locale-table-fetch:
    python3 scripts/locale_table.py --fetch

locale-table:
    python3 scripts/locale_table.py --write

locale-table-check:
    python3 scripts/locale_table.py

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

# A count of instructions under `callgrind` does not move between two runs, so
# unlike a time it can fail a build. The CI `instructions` job runs this; the
# ceilings and how a budget converts to one live in the script. Needs valgrind.

# instruction-count budgets: each counted operation against its ceiling.
instructions:
    python3 scripts/instructions.py

# The download of the Main instance against its ceiling and its budget
# (`ambre-spec-budgets.md` section 5). Builds the release Trunk ships.
size:
    trunk build --release
    python3 scripts/wasm_size.py

# The `db` feature flips the `cfg_attr(not(feature = "db"), ignore = ...)` gate
# off so the otherwise ignored tests run. Bring services up first with
# `just services-up` (CI declares them as `services:` instead).
#
# `test-helpers` is the second half: `redis_ops::lua_tests` is
# `cfg(all(test, feature = "test-helpers"))`, so its six Lua/clock tests --
# including the five that verify `SRV-006`, the strict per-client clock -- did
# not even compile under `--features db` alone. 156 tests were collected where
# 162 exist.

# db tier: server integration tests (needs postgres/redis/garage). S3_ENDPOINT
# defaults to the garage of `services-up`; the media round trip fails without it.
test-db:
    S3_ENDPOINT="${S3_ENDPOINT:-http://localhost:9000}" cargo test -p ambre-server --features db,test-helpers

# One path per seed (`CALL-053`): every test of the SFU simulation in 20
# concurrent processes, in the release build. The native tier runs the same
# guard in three processes.
sim-paths:
    AMBRE_SIM_PROCESSES=20 cargo test -p ambre-sfu --release sim::paths -- --nocapture

# browser tier: the Trunk build of the whole application, served in front of a
# relay and driven through WebDriver (`crates/ambre-belt`). It needs the db-tier
# services, a Chrome and its chromedriver (`CHROMEDRIVER`, `BELT_CHROME`). One
# test at a time: each starts a relay and a browser of its own. The call test
# starts the SFU binary instead, and two browsers on fake capture devices. The
# TURN test also starts coturn (`BELT_TURNSERVER`) behind a TCP front on port
# 443 (`BELT_TURN_PORT`).
test-browser:
    trunk build
    cargo build -p ambre-server
    cargo build -p ambre-sfu
    S3_ENDPOINT="${S3_ENDPOINT:-http://localhost:9000}" cargo test -p ambre-belt --features browser -- --test-threads=1

# The browser tier on Firefox: the same belt, driven by headless Firefox
# through geckodriver (`GECKODRIVER`, `BELT_FIREFOX`). A test that pauses a
# worker through the DevTools Protocol is ignored there. Headless Firefox on
# Linux makes its WebGL2 context through an X display, so without `DISPLAY`
# the run takes a virtual one from `xvfb-run`.
test-browser-firefox:
    trunk build
    cargo build -p ambre-server
    cargo build -p ambre-sfu
    if [ -n "${DISPLAY:-}" ]; then x=""; else x="xvfb-run --auto-servernum"; fi; S3_ENDPOINT="${S3_ENDPOINT:-http://localhost:9000}" $x cargo test -p ambre-belt --features firefox -- --test-threads=1

# `cargo test` runs each wasm test module through `wasm-bindgen-test-runner`,
# named as the target's runner; the pinned nightly + rust-src
# (rust-toolchain.toml) supply `-Z build-std`. The runner must be the version of
# `wasm-bindgen` that Cargo.lock pins: the runner refuses a module another
# version wrote. `cargo install wasm-bindgen-cli --version <that version>`.
#
# The driver is headless Chrome. `CHROMEDRIVER` names its chromedriver when set,
# and `chromedriver` on PATH otherwise: left unset, the runner would take the
# first driver it finds, and it tries geckodriver first.
# `WASM_BINDGEN_TEST_WEBDRIVER_JSON` may name a capabilities file, for example a
# Chrome binary off the default path. `WASM_BINDGEN_TEST_ONLY_WEB` skips a test
# module not configured for the browser, so no test runs under Node.
wasm_test_env := 'RUSTFLAGS="-Awarnings -C debuginfo=0" CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner CHROMEDRIVER="${CHROMEDRIVER:-chromedriver}" WASM_BINDGEN_TEST_ONLY_WEB=1'

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
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-trame
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-chrome
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-toile
    # The Prose unit tests are host-only (proptest, shaped fixtures): the chrome tests alone.
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-prose --test chrome_panel --test chrome_ui
    # The Murmure unit tests are host-only too: the chrome relay test alone.
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-murmure --test chrome_relay
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-logic
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-app
    # The converter loader, on modules written by hand (`ARCH-079`).
    {{wasm_test_env}} cargo test --target wasm32-unknown-unknown -p ambre-convert --features loader --lib

# Host only: the wasm tier is `-Z build-std`-heavy and Chrome-bound, while
# `cargo-llvm-cov` and proptest run far better on host.

# Coverage (native tier): lcov baseline, non-blocking in CI.
cov:
    cargo llvm-cov --workspace --lcov --output-path lcov.info

# Mutation testing on the host tier (`ambre-spec-testing.md` section 2). A wasm
# or browser test costs a std rebuild or a Chrome start per mutant, so neither
# tier runs. `.cargo/mutants.toml` holds the scope and each exclusion with its
# reason. `shard` splits the run as `k/n`, `k` from 0: the CI `mutants` job runs
# each shard on its own runner. A surviving mutant exits non-zero, and its diff
# lands under `mutants.out/`. `flags` go to cargo-mutants as written: CI passes
# `--in-place`, which builds in the restored `target/` rather than in a copy.

# mutation testing: the mutants of the configured scope, one shard of them.
mutants shard="0/1" *flags:
    cargo mutants --shard {{shard}} --no-shuffle {{flags}}

# The mutants of the lines this branch changes against `base` alone. The CI
# `mutants-diff` job runs it on a pull request and blocks on a survivor.

# mutation testing: the mutants of the lines changed since `base`.
mutants-diff base="origin/main" *flags:
    git diff {{base}}...HEAD > mutants.diff
    cargo mutants --in-diff mutants.diff {{flags}}

# A spec states what and why; the source states how (`ambre-spec-template.md §0`).
# Shapes are the contract and stay -- `struct`, `enum`, trait and public
# signatures. Function bodies are a second, uncompiled copy that drifts: a
# `trame-ui` snippet contradicted its own sentence, and `proptest!` bodies
# described a suite that did not exist. This makes the rule mechanical.

# no implementation in the specs: fails on any function body in a ```rust fence,
# on Rust shapes left in a non-rust fence, on a disallowed fence tag (lua, ...),
# on an unterminated fence (ambre-spec-template.md section 0, Fences rule), on a
# rule cited outside its application, and on a test that cites a rule whose
# `Mechanism` does not name it (the defects listed in spec_code_citations.allow aside).
spec-code:
    python3 scripts/spec_code.py

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
# form no other gate reads. The two files weigh megabytes and only a tool reads
# them, so git never carries them: `index-write` builds them under `target/specs/`,
# and `spec-context` and `spec-code` build the same parse in memory.

# contract index and graph: build them in memory, fail on an edge resolving to nothing.
index:
    python3 scripts/contract_index.py

# Build the index and the graph into `target/specs/`, for a tool that reads them.
index-write:
    python3 scripts/contract_index.py --write

# The primitive gate: every site outside a test whose type holds a bare
# primitive is typed, or named by one line of
# `specs/1-Foundations/ambre-primitives.tsv` with a category and a reason
# from a closed set (`ARCH-033`). No category means "to type later". A line
# that names no site fails too (`ARCH-034`). It prints the lines per category.
#
# The gate reads the code through rustc. It builds the driver of
# `crates/ambre-primitives/driver` on the pinned nightly, which needs the
# `rustc-dev` component. Then it reads the compile lines of `lint` and
# `lint-wasm` from this file and runs each as `cargo check`, the driver in
# place of rustc. So the gate reads each unit clippy lints, under the same
# target, `cfg` and feature set (`ARCH-050`). Clippy holds the wrapper of the
# lint, so the gate makes cargo compile each workspace unit again; the
# dependencies stay fresh in the shared target directory. A `.rs` file no
# compile reads fails.
#
# `--unwrap-strict` runs the strict unwrap report on the same records, and
# prints both verdicts whatever the first says. CI runs this recipe alone,
# since it holds both.
#
# The same run holds the crate boundaries (`ambre-spec-architecture.md`
# section 2). The driver refuses a resolved path that reaches what its part of
# the workspace must not, and the judge reads the graph `cargo metadata`
# resolves: two instances, the network from the worker, one wire definition.
# The driver also reads types and bodies: a generator's reads, a request id, a
# payload in a relay log. The judge counts the launcher's wiring over every
# unit: one engine start, a relay that drains, a fatal latch per failure.
primitives:
    cargo run -q -p ambre-primitives -- check --unwrap-strict

# The unwrap report (`ARCH-073`). It refuses a newtype unwrapped early inside
# a body, unless a line of `specs/1-Foundations/ambre-unwrap.tsv` names its
# key with a count and a reason. In strict mode, an unlisted key, a line over
# its count and a stale line all fail. CI runs it through `primitives`; this
# recipe runs it alone, on a compile of its own.
unwrap:
    cargo run -q -p ambre-primitives -- unwrap --strict

# The driver of the primitive gate: its format, its clippy and its fixture
# tests, which compile fixture crates through it (`ambre-spec-testing.md`
# section 2). It is a workspace of its own, which no line of `lint` reads, so
# this recipe holds it. Cargo reads its configuration from the root of the
# disk (`-C /`), since the `build-std` key of `.cargo/config.toml` would link a
# second `std` beside the one of `rustc_driver`. The recipe starts at the root
# of the repository, so rustup picks the pinned toolchain and no line names
# one (`ARCH-075`). Needs the `rustc-dev` component.
primitives-driver:
    cargo fmt --manifest-path crates/ambre-primitives/driver/Cargo.toml --check
    cargo -Z unstable-options -C / clippy --release --all-targets --manifest-path "{{justfile_directory()}}/crates/ambre-primitives/driver/Cargo.toml" -- -D warnings
    cargo -Z unstable-options -C / test --release --manifest-path "{{justfile_directory()}}/crates/ambre-primitives/driver/Cargo.toml"

# Remove the lines that name no site, after a value was typed. No recipe adds
# a line: a new one is a hand edit a reviewer reads (`ARCH-035`).
primitives-prune:
    cargo run -q -p ambre-primitives -- --prune

# How a spec is built, not what it quotes: prose share and line length, both
# capped for every spec, production markers in a feature document, the schema
# document's budgets, and every container built in the code present in the
# schema table.

# spec shape: prose share (words), line length, feature-document markers, schema
# budgets, containers.
spec-shape:
    python3 scripts/spec_shape.py

# A contract row is seven columns, and four of them answer a reviewer rather than
# a reader: where the rule comes from, which mechanism holds it, what that leaves
# open, and where it stands. They are why the table is trustworthy, and why one
# row wraps over four screen widths. This prints the same rows reduced to the
# three a reader asks for, from the parser every gate shares (`scripts/corpus.py`),
# so the view can never state a rule the ledger does not. It writes nothing: a reading view
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
    python3 scripts/prose.py

# Each application freezes, under its compiled `SCHEMA_VERSION`, the key set and
# the value types a build writes. A belt in the crate holds a build against that
# text, and the belt alone is weak: regenerating the section in place turns it
# green with the version never raised, while every peer on the old version reads
# a document it cannot parse. A section is the record of what one version wrote,
# so this refuses any section the base revision carries whose body moved. Pass
# `--base` to name another revision. A section binds only from the first release
# on: while no `git tag` matches `v[0-9]*`, no peer holds a document written
# under any section, so this reads no base and holds the pairing of each freeze
# with its belt alone (`SCHEMA-019`). A shallow clone with no tag at all cannot
# tell a release from none, so this refuses it: run `git fetch --tags --force`.

# append-only schema freezes: a released shape section never moves.
schema-freeze:
    python3 scripts/schema_freeze.py

# A frozen artifact outlives its generator, so the tree of a later commit cannot
# show where it came from. Its freeze commit can: the freeze must sit in the bump
# commit (`DOCFILE-008`), its bytes must be the outgoing writer's run from a seed
# (`FIXTURE-005`), and that writer must read no file (`FIXTURE-032`). A rerun
# builds the generator at an old commit, so only the artifacts the base lacks are
# read. Pass `--all` to read every one, and `--base` to name another revision.

# each new frozen artifact traced to its seed at the commit that froze it.
freeze-provenance:
    python3 scripts/freeze_provenance.py

# The boundaries neither rustc nor cargo reads, held by a scan of the text: a
# stylesheet, a page, a string, and the code rules that still wait for the
# driver of `just primitives`, which holds every other crate boundary.
crate-boundaries:
    python3 scripts/crate_boundaries.py

# No format converter in the Main instance or the Sync worker: each lives in
# the converter module, fetched on the first conversion (`ARCH-078`). Read from
# the graph cargo resolves for each instance on the wasm target.
converter-boundary:
    python3 scripts/converter_boundary.py

# The checks in `scripts/` guard every Rust comment, every document and every
# contract row, and nothing guarded the checks. Three defects shipped in the
# prose one, and the worst produced no symptom: a table row left the reader's
# result, so the hard rules never read it. A check that reads nothing and a
# check that is satisfied look alike from outside, which is what these hold,
# one test file per script. `test_corpus.py` holds one fixture document that
# every gate reads through `scripts/corpus.py`, the same way.

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
    python3 scripts/spec_refs.py

# What the product does today, one row per feature document. The roadmap holds
# what is left to do, and a feature built by three stories across two epics
# appears nowhere whole in it. The inventory carries no fact of its own, so it is
# generated: this regenerates it in memory and fails when the file on disk
# differs, which is what stops a spec edit from leaving a stale product surface.
# It also reads the rows against the header, and fails on a document that claims
# `implemented` beside a row that is not built.

# feature inventory: generated from the feature documents, and never stale.
inventory:
    python3 scripts/feature_inventory.py

# Rewrite each application's inventory after a feature document changed.
inventory-write:
    python3 scripts/feature_inventory.py --write

# The toolchain is one file: `rust-toolchain.toml`, a dated nightly that moves by
# a roadmap story alone (`ARCH-075`). A toolchain named in any other file builds
# part of the workspace on another compiler, and a bump would miss it. `fuzz/`
# inherits the root file through the directory lookup of rustup. The script
# lists what it refuses.

# toolchain pin: no file but `rust-toolchain.toml` names a toolchain or a channel.
toolchain-pin:
    python3 scripts/toolchain_pin.py

# A tier result speaks for the image `docker-compose.yml` pins and for no other
# (`ambre-spec-testing.md section 4`). A tag such as `8-alpine` moves to each new
# release under the same text, and the services of a CI job repeated the tags by
# hand. The script refuses an image with no digest, a tag that is not one
# release, and a CI service that differs from the compose image (`ARCH-087`,
# `ARCH-088`).

# service pins: each image names one version and one digest, and CI runs the compose images.
service-pins:
    python3 scripts/service_pins.py

# Supply chain + capability wall. `deny.toml` sets `[graph] all-features = true`,
# so optional dependencies (the `fixtures` feature's `ambre-crypto` edge) are part
# of the graph being checked rather than invisible to it. No network build: this
# reads `Cargo.lock` and the advisory database only. A wrapper no edge of the
# graph matches is an error, not a warning: it grants a capability to a crate
# that no longer takes it, so the day one does no one reviews the edge.

# cargo-deny: advisories, licences, sources, and the ZK capability bans.
deny:
    cargo deny check --deny unused-wrapper

# cargo-fuzz: each untrusted reader, for `seconds` apiece (`ambre-spec-testing.md
# section 2`). The targets live in `fuzz/`, a workspace of its own, so the
# sanitizer flags never reach the main build. A crash stops the run and leaves
# its input under `fuzz/artifacts/`.
#
# The target is the host triple of the pinned `rustc`, never the default of
# `cargo-fuzz`: that default is the triple `cargo-fuzz` itself was built for.
# The prebuilt binary CI installs is a musl one, and a musl target links libc
# statically, which the address sanitizer refuses when `build-std` rebuilds
# `core`.
fuzz seconds="60":
    cd fuzz && host="$(rustc -vV | sed -n 's/^host: //p')" && for target in wire_decode docfile formula markdown sealed trame_file toile_file; do cargo fuzz run --target "$host" "$target" -- -max_total_time={{seconds}} -rss_limit_mb=2048 || exit 1; done

# Local convenience: bring the db-tier backing services up, the object store
# prepared.
services-up:
    docker compose up -d postgres redis
    just garage-up

# The object store alone: the Garage of `docker-compose.yml`, with its layout,
# the dev key, the bucket and the CORS rule (`scripts/garage_init.sh`). CI runs
# this recipe; postgres and redis are services there.
garage-up:
    docker compose up -d garage
    bash scripts/garage_init.sh

# Local convenience: tear the db-tier backing services down.
services-down:
    docker compose down
