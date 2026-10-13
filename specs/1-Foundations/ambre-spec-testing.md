# AMBRE -- SPEC: TESTING

**Type:** Horizontal (Foundation -- Quality)
**Implementation:** implemented

> **How Ambre is verified, and how well. Normative.** When another document says how a component is
> tested, this one wins. One file holds the method, because two files disagree.
>
> What a rule looks like is `ambre-spec-template.md`. What every rule's status is today is
> `ambre-contract-ledger.md`, generated. This document holds neither.

---

## 1. DOCTRINE

* **Reading code does not scale as evidence.** A review forms an opinion; a verification proves a
  behaviour.
* **A mechanism earns its place on one condition:** it permanently removes a class of code from the
  reading queue. Otherwise it is one more thing to maintain and to read.
* **Rust makes that trade cheap.** Types, the lint wall and property tests move whole categories out
  of human attention for good.
* **Where no mechanism exists**, attention is spent deliberately and the document says so. Each
  such place is an obstacle of section 3, and a row claiming it names it.

**What a test may claim.**

* **A vacuous property proves nothing.** `prop_assert!(result.is_ok())` is not a property, and
  neither is a test that reimplements the function under test.
* **Demand two independent paths.** Encode against decode. Incremental against full rebuild. Peer A
  against peer B. A property relating two paths is a specification sentence, readable in three
  seconds.
* **A found bug becomes a named regression test**, never a new property.

**What the reviewer reads.**

| Read | Do not read |
| :--- | :--- |
| public signatures, property names, mutant reports, snapshot diffs, the `Mechanism` column | function bodies, and individual assertions a mechanism already covers |

---

## 2. THE MECHANISM LADDER

Ordered by cost of ownership, not by sophistication. **Adopt from the top:** an invariant held by the
compiler needs no test, no run and no reading.

| Mechanism | Effect | Adopt when |
| :--- | :--- | :--- |
| Newtypes, enums, typestate, `#[must_use]` | the invariant holds if it compiles -- permanent, zero maintenance | always, and first choice for every ZK-boundary invariant |
| Lint wall: `[workspace.lints]`, `deny(unwrap_used, panic, indexing_slicing)`, `#![forbid(unsafe_code)]` in **every** crate | removes a whole scanning category from review | always. The "no `unsafe`" pillar is a claim until the lint enforces it crate by crate |
| `cargo-deny`: advisories, licences, duplicate versions, capability bans | the supply chain of a crypto product, checked by machine | always |
| A gate in `scripts/`, run by a `just` recipe in the CI `lint` job | holds what the compiler cannot see: the corpus, the prose, and a committed artifact that must not move | a rule about the tree rather than the code. Its own tests run under `just test-scripts`, because a gate that reads nothing and a gate that is satisfied look alike |
| `proptest` | universal rules over an unenumerated input space | pure logic and reference models: the formula engine, CRDT convergence, cache against rebuild |
| `cargo-fuzz` plus `arbitrary` | robustness against hostile input | every decoder fed by the relay or by a file. The relay is untrusted by design, so this is not a hardening extra |
| `cargo-mutants` | verifies the tests themselves -- a surviving mutant is unverified code | periodically, starting with the crates whose belts were generated fastest |
| `cargo-llvm-cov` in diff mode | are the added lines exercised at all | as a CI gate, once the informative baseline exists |
| `criterion` | a time on the machine of the author, for the figure a budget row quotes | an operation carrying a numbered budget in `ambre-spec-budgets.md`. A time on a shared runner moves by tens of percent between runs, so it never fails a build |
| `callgrind`, a count of instructions with a CI ceiling | a performance regression never appears in a diff, and a count does not move between runs | every operation carrying a numbered budget in `ambre-spec-budgets.md`. The ceiling is the budget converted at the rate `scripts/instructions.py` states, and the CI `instructions` job fails an operation above it. It counts the native build: the phone figure stays a conversion |
| `insta` snapshots | read consequences instead of code; reveals unintended blast radius | deterministic structured output: `RecordingSurface` op sequences, wire encodings. **Acceptance stays human** -- `cargo insta accept` accepts blindly |
| `loom`, `miri` | interleavings under the C11 model, undefined behaviour under Stacked and Tree Borrows | **only if hand-written `unsafe` or lock-free atomics appear.** Measured 2026-08: zero `unsafe` in 17 crates, and atomics are flags and counters. Not adopted |

---

## 3. THE REGIME PER COMPONENT SHAPE

**The regime follows the shape of the component, never its importance.** A new application adds a row
here only when it brings a shape this table does not hold.

| Component shape | Regime | Mechanism |
| :--- | :--- | :--- |
| A pure engine: parser, VM, dependency graph | pure properties | `proptest` on host: order independence, exactness, parser round trip, cycle detection. No I/O, thousands of cases |
| CRDT state | reference model | 2-peer convergence through the application's mock sync handle plus `apply_remote`; every incremental cache asserted against a full rebuild |
| Cryptography | strict | round trip, tampering, zeroize, `proptest` |
| The ZK boundary | typing first, negative tests second | make plaintext **unrepresentable** at the transport boundary; what the types cannot hold becomes a test asserting an absence |
| A service that **is** persistence | model-based, over the real services | never mock the database: for such a service, mocking tests the mock. `#[sqlx::test]` gives one database per test. Order of operations produces the bugs, and order is what nobody enumerates by hand |
| A decoder fed by a user file or by the relay | totality first | a round trip, a malformed case, a random-bytes case, then a `cargo-fuzz` target. Totality is not optional |
| A GPU pipeline | assisted review plus a visual check | `qa-manual` through `/run`, whose first row declares its obstacle below. Surface minimisation keeps this small: any rule that can live in pure Rust must live there |
| DOM glue | extraction first, then the browser | the decision logic on host (section 5.A), the rest in the wasm tier or in a driven browser. Never `qa-manual`: the obstacle is lifted, below |

**CRDT convergence is mandatory.** Every collaborative CRDT state ships a property test of
convergence. It generates operation sequences with `proptest`, interleaves them across two peers or
more, and asserts identical final states once every delta has been exchanged.

**An obstacle is why no mechanism reaches a rule, and it is named.** A `qa-manual` or a
`human-review` row claims a gap that cannot close. That claim is a fact about the tooling, and
tooling moves. So the row's `Gap` opens on `` unheld -- `<obstacle>` -- ``, one obstacle of the
table below. A row a mechanism half holds opens on `` partial -- `<obstacle>` -- `` instead.

**The three obstacles.**

| Obstacle | What no mechanism reaches | Lifted by |
| :--- | :--- | :--- |
| `intent` | what a value means, or why a line exists: the right code and the wrong one have one shape, so no check tells them apart | -- |
| `outside-the-tree` | a claim made where no check runs: a pull request, the commit an artifact was owed in, the origin of committed bytes | -- |
| `dom-glue` | a listener, a browser API, the shell reading a level each frame | `The browser-driven belt` Story (`ambre-roadmap.md` E9) |

* **`Lifted by` names the Story that brings the mechanism, or `--`.** A row citing a lifted obstacle
  fails `contracts.py`, the day the Story is written, not the day it ships. Its gap can close, so
  it is debt: `untested` or `partial`, and it lands with a Story.
* **A new obstacle is a row here first.** A row citing a name the table does not carry fails.
* **An obstacle lives while a row needs it.** A standing one no row cites fails, since it would
  excuse the next row without an argument. A lifted one goes once its Story ships.
* **The day a seam or a tool reaches an obstacle**, its Story goes in `Lifted by`. The gate then
  lists every row to requalify, so no reader has to remember them.

---

## 4. THE THREE TIERS

| Tier | Command | Holds | Cost |
| :--- | :--- | :--- | :--- |
| **host** | `just test-native` | the default tier. Pure logic, zero external dependency. The Logic thread, the Sync pipeline, every formula and CRDT handler, and the render feed | fast |
| **db** | `just test-db` | `ambre-server` integration behind the Cargo feature `db`. `docker compose up postgres redis minio`, then `#[sqlx::test]` per test | medium |
| **wasm** | `just test-wasm` | every `cfg(target_arch = "wasm32")` surface: DOM and Leptos, GPU. `wasm-pack test --headless --chrome` | slow: `-Z build-std` rebuilds `std`, then Chrome starts |

**The db tier is a Cargo feature, never a bare `#[ignore]`.** The engraved pattern is
`#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]`. A bare
`#[ignore]` is a test that no job runs and no reader counts.

**A cited mechanism must run in CI.** A test name behind a Cargo feature no job enables
claims as much as a name that resolves to nothing. `contracts.py` holds the first half. Nothing
holds the second, and it went unseen for six months.

The five tests that are the whole
`Mechanism` cell of `SRV-006` sat behind `--features test-helpers`, which no CI job enabled.
Eight `ambre-toile` PDF tests sat behind `--features pdf`. Both features are in `just test-db` and
`just test-native` now, which is where local and CI parity is defined.

`just test-wasm` covers `ambre-trame`, `ambre-chrome`, `ambre-toile`, `ambre-logic`, `ambre-prose`
and `ambre-app`.

---

## 5. THE THREE SEAMS

All three seams do one thing: push the untestable boundary as close to the hardware as possible, then
unit test everything on the pure side. The Logic thread needs no seam -- it is pure Rust already, which is
a designed property of the three-thread stack (`ambre-spec-architecture.md section 2`).

### A. The DOM seam

A Leptos component mixes three things, and only one of them is pure Rust.

| Part | Nature | Where it is tested |
| :--- | :--- | :--- |
| The rendering: `view! { ... }` | irreducibly wasm | the wasm tier |
| The event wiring: `on:input`, clipboard | irreducibly wasm | the wasm tier |
| **Decision logic**: validating input, deriving state, mapping an event to view state | pure `data -> data` | host, after extraction |

* **Extraction** moves the third part into a free function in a non-gated module, which the component
  calls. The component shrinks to a shell: DOM input, pure function, signal.
* **Extraction is an optimization, not a rule.** The wasm tier already covers presentation logic end
  to end, and it is a first-class way to test the chrome.
* **Prefer it where it is cheap**, because the wasm tier is slow, and coverage and `proptest` both
  run far better on host. Skip it for thin glue not worth a refactor.
* **What never extracts:** anything intrinsically DOM. Popover positioning through
  `getBoundingClientRect`, focus, real keyboard selection. Those stay in `wasm-bindgen-test`.

| Applied to | Extracted to host | Left in wasm |
| :--- | :--- | :--- |
| `ambre-chrome` | `command::key_from_dom`, the DOM-key classification; `context_menu::place_menu`, the viewport-flip geometry; `passphrase::vet`, the one decision of the seal prompt | the modifier bits off a `KeyboardEvent`, and the `getBoundingClientRect` read |
| `ambre-app` | nothing: the crate is `cfg(wasm32)` only, so its pure helpers run as inline `#[wasm_bindgen_test]` | `route::parse`, `host::base64_*`, `FrameHost::tick_fps` |
| `ambre-trame` | geometry, hit testing, selection, editor state, the draw-call sequence | the DOM chrome only |

> `#[wasm_bindgen(start)] fn main` is gated `#[cfg(not(test))]`, so the test harness's generated entry
> point does not collide with it.

### B. The GPU seam

Rendering splits across the `DrawSurface` boundary in `ambre-draw`.

| Half | Where it lives | How it is verified |
| :--- | :--- | :--- |
| **What to draw** | the application render layer, which never names wgpu. It emits `fill_rect`, `stroke_line`, `draw_glyphs`, `push_clip` | `RecordingSurface` (`ambre-draw`, feature `testing`) on host, no GPU. Assert the ordered `DrawOp` sequence: geometry, colours, clip balance, glyph-run counts. **This is where the vast majority of render defects live** |
| **How to draw it** | `ambre-canvas`: `QuadRenderer`, `WgpuSurface`, `GlyphAtlas` -- the upload and the WGSL shaders | the device calls are not unit-tested, by decision: they need a real `wgpu::Device`. Thin, stable plumbing, validated visually through `/run`. The CPU work beside them is host-tested, below |
| **The page** | `KrillaSurface`, the PDF surface | a snapshot over simple primitives, plus byte determinism: two exports of one unchanged document produce identical bytes |

A headless software adapter and golden-image readback are both reserved for a precise critical need.
The first is driver-dependent, the second brittle under antialiasing drift. The default is the
`DrawSurface` contract plus a visual check.

The "how to draw" half carries CPU work that needs no device, and it is extracted. `GlyphCache` packs
the atlas, repacks it on overflow and queues the uploads. `fill_mesh` and `stroke_mesh` tessellate a
path into screen-space vertex buffers. `ImageCache` orders the eviction. Each is tested on host like
the DOM seam's decision logic. Only the upload and the shader stay behind the device.

### C. The transport seam

The network is the third untestable boundary: the browser's `fetch`, `WebSocket` and timers, and on
the relay, a second instance. Both sides follow the same rule.

| Part | Nature | Where it is tested |
| :--- | :--- | :--- |
| The browser call: `fetch`, `AbortSignal`, `WebSocket::new`, `setTimeout` | irreducibly wasm | a thin shell, held by the wasm tier or by review |
| **The request policy**: renew once on an expired token, never on a refused one, the timeout, the error mapping | pure, behind a send trait | host, with a scripted transport |
| **The socket lifecycle**: open, close codes, backoff and its reset, the reconnect replay | a state machine over an injected clock | host, by stepping the clock |
| **The relay across instances**: broadcast, presence, session limits and revocation | two `AppState`s on one Postgres and one Redis | the db tier |

* **The send trait is the precedent already in the crate.** `MediaHttp` and `ShareHttp` put the media
  and sharing calls behind a trait, and the socket has an in-memory `MockWsTransport`. The request
  policy sits above `http::Transport`, which only the browser's `fetch` implements outside tests.
* **The clock is a value, not a call.** The lifecycle reads time from a parameter, so a test advances
  it without sleeping. The timer shell only turns a deadline into a `setTimeout`.
* **Two instances means two states, not two processes.** Each `AppState` owns its own hub and
  connection tables. Sharing the stores is exactly what the deployment shares.
  `test_helpers::second_instance` builds the second one beside any test state.
* **What never extracts:** the browser's own behaviour under a real network -- a proxy, a dropped
  TCP connection, a CORS refusal. That stays with `The browser-driven belt`.

---

## 6. REALIZATION PER CRATE

One row per crate: what runs, what stands in for the world, and the belt an increment lands with.

| Crate | host | wasm | Seams | Belt |
| :--- | :--- | :--- | :--- | :--- |
| `ambre-core` | yes | -- | -- | types, plus the `.ambre` container: every header failure (truncated, bad magic, `UnsupportedVersion` in both directions, `WrongDocType`), byte soup without a panic, and the migration walk driven by synthetic steps, because no second version exists to exercise it. The regression belt over real files is the fixture corpus (`ambre-spec-fixtures.md`) |
| `ambre-crypto` | yes (`proptest`, `criterion`) | -- | -- | round trip, tampering, zeroize, **plus human review**. The file key adds its own: the derivation is deterministic per salt, a `FileKey` and a user key from the same inputs differ, and a sealed round trip fails closed on a wrong passphrase (`DOCFILE-016`) |
| `ambre-protocol` | yes | -- | -- | postcard wire round trip |
| `ambre-bus` | yes (`tests/host.rs`) | -- | `CommandSink`, the trait a controller sends through | Logic/Sync envelope postcard round trips, with the `id` echo preserved. The round trip is a sample rather than an enumeration: 5 of 12 `SyncCommand` variants and 4 of 19 `LogicEvent` variants (`BUS-003`) |
| `ambre-logic` | yes | -- | mock queues on both sides | typed dispatch: forward, field preservation, drop on an unknown document, the role gate on a viewer and on an embedded child, the service route and its correlated failure |
| `ambre-sync` | yes | -- | `MockWsTransport`, an in-memory socket; `MockBlindRelay`, an in-process relay; `LogicSink::queue` | four, because this is the boundary the product rests on. **Round trip** per message type. **Tampering**: one mutated byte, a wrong key, a wrong `document_id`, a wrong signature -- each fails closed. **Ordering**: an update ahead of its snapshot is quarantined, released in clock order, and the quarantine is bounded. **Replay**: a non-increasing counter is dropped. The two sealed-file handlers are the exception to the boundary framing -- they reach no relay, and their belt is a round trip at the real work factor plus one failure shape for a wrong passphrase and a tampered file |
| `ambre-canvas` | yes | GPU not unit-tested, by design | `RecordingSurface` | `TextCache` hit and LRU eviction, `GlyphAtlas` eviction, the dirty flag skipping the batch build, font fallback on an absent codepoint, and the compiled icon blob covering every SVG of `assets/icons/` with a non-zero vertex count per icon |
| `ambre-commands` | yes | -- | -- | registry lookup, keybinding resolution, derived `CommandState` |
| `ambre-trame` | yes (`proptest`) | yes (`tests/chrome_ui.rs`) | `MockSyncHandle`, `TrameState::with_peer`, `RecordingSurface`, a `TrameView` on a detached pipe and a test clock | **CRDT convergence** over generated interleavings. **Formula properties**: totality, exactness, order independence over generated trees. **Cache against rebuild** on the `SheetIndex`. A **geometry round trip**: drawn rect against hit test, under zoom and variable metrics. A `RecordingSurface` assertion for anything painted. **Per feature**: a view feature lands with a positive case, a no-op guard, a convergence case and a proof that it changes no data; a mutation feature lands with the permutation assertion, a header-pin case, a no-delta case and a references-follow case |
| `ambre-chrome` | yes (`key_from_dom`, `place_menu`, `vet`) | yes (`tests/chrome_ui.rs`) | section 5.A | the reactive store over real signals, the toast ARIA live region, the shell canvas cell, and the menu flip on viewport overflow |
| `ambre-app` | -- | yes (inline) | -- | `route` round trip and malformed fallback, the base64 codec, the FPS sliding window. A re-render is read after `leptos::task::tick`, and a width by mounting in a frame of that width, since a width query reads the frame's viewport |
| `ambre-server` | yes | -- (db tier) | `#[sqlx::test]`, a local Valkey | ACL per role **and per membership**, Ed25519 accept and refuse, the Redis Lua clock cases (valid, mismatch, first update, two concurrent clients), the rank clamp on both the granted and the target role, the size cap on both durable paths, and a postcard round trip per message variant -- the document plane only: the four channel variants and the share notice have none |
| `ambre-toile` | yes | yes (`tests/chrome_ui.rs`, `tests/chrome_effects.rs`) | `ToileState::with_peer`, `RecordingSurface` | CRDT convergence, and the spatial index against a full rebuild |
| `ambre-draw`, `ambre-pdf` | tooling | -- | they **are** the seam | `RecordingSurface` is exercised by every consumer |
| `ambre-prose` | yes (`proptest`, `criterion`) | yes (`tests/chrome_panel.rs`) | `ProseState::with_peer`, `RecordingSurface` | CRDT convergence of text and annotations, placement properties over generated documents, and a `RecordingSurface` assertion for anything painted. The comment panel is mounted in Chrome |
| `ambre-murmure` | -- | -- | -- | none yet: a stub. The belt it lands with: CRDT convergence of a channel across its documents, a forged delta refused by the authorship check, a false final snapshot caught by the second client, and eviction held to its byte budget |

**A belt is a floor, not a list.** It says what an increment cannot land without. The tests that
verify a rule are named in that rule's `Mechanism` cell and nowhere else. A list of intended
tests has no join to the suite and no way to fail.

**Where an application needs a harness of its own**, that harness is a feature. It gets a document in
the application's folder, cited from this table. `ambre-spec-fixtures.md` is the shape, and it sits in
`1-Foundations/` because two applications need it.

---

## 7. CI

`.github/workflows/ci.yml`, one job per tier, each calling the matching `just` recipe so local and CI
run the same command.

| Job | Runs |
| :--- | :--- |
| `lint` | `cargo fmt --check`, `cargo clippy --all-targets -D warnings` on host with every feature on, and the five corpus scripts |
| `test-native` | the host tier, plus `ambre-server` on the native target |
| `test-db` | postgres, redis and minio as services, with `--features db` |
| `test-wasm` | `just lint-wasm`, the denied lints on the wasm target, then `wasm-pack test --headless --chrome` |
| `deny` | `cargo deny`: advisories, licences, sources, and the capability bans |
| `coverage` | `cargo-llvm-cov` over the host tier, non-blocking. An `lcov` baseline as an artifact |

The wasm `-A warnings` rustflag is scoped to `[target.wasm32-unknown-unknown]`, so host clippy keeps
every warning. Every native job installs `libfontconfig1-dev`: the font stack links system fontconfig.

---

## 8. MECHANISM GAPS

What section 2 prescribes, against what is wired. Measured 2026-08. Each absence is a decision rather
than an omission. Epic `E9` of `ambre-roadmap.md` schedules them, and the milestone of each story says
whether it blocks the current MVP.

| Mechanism | State | Gap |
| :--- | :--- | :--- |
| Type-level invariants | **the strongest area**: the id newtype macro, `PublicKey`, `Ciphertext`, `SecretBytes`, `SecretString` | not the mechanism, the *statement*: nothing says which invariants these types hold, so the guarantee is invisible |
| `#![forbid(unsafe_code)]` | **wired** | `unsafe_code = "forbid"` in the workspace lint table, so all 17 crates refuse an `unsafe` block |
| `[workspace.lints]` | wired | every crate inherits it (`ARCH-032`). It denies `missing_errors_doc`, `missing_panics_doc` and the panic lints (`ARCH-028`); no `pedantic`. Clippy reads the wasm-only code in the `test-wasm` job |
| `cargo-deny` | **wired** | advisories, licences, sources, and the capability bans that mechanise five ZK rows. Negative-tested |
| `proptest` | wired | `ambre-crypto`, the Trame formula engine, 2-peer convergence on Trame and Toile, and the Toile index against rebuild |
| `cargo-fuzz` | wired | four targets in `fuzz/`: the wire decode, the `.ambre` container, the formula parser and the Markdown reader, one minute each in the CI `fuzz` job. The sealed open path and the Trame and Toile file readers have none |
| `cargo-mutants` | absent | the whole suite, never adversarially checked |
| `cargo-llvm-cov` | wired, non-blocking | an `lcov` baseline only. No diff-mode gate on added lines |
| `criterion` | benches in `ambre-crypto`, `ambre-toile` and `ambre-murmure` | a desktop figure only. **The mobile measurement the targets are written against has never been taken** |
| `callgrind` | wired, three budgets | the CI `instructions` job holds the Toile body commit, the content walk of a busy Toile board and the Prose text layout of a paragraph under their ceilings, and the bound the Toile diff cap allows. Every other budget row has no ceiling yet |
| `insta` | absent | candidates: `RecordingSurface` op sequences, wire encodings, and the `KrillaSurface` primitives |
| `loom`, `miri` | **not adopted, by decision** | zero `unsafe`, and atomics are flags and counters. Re-check with `rg 'unsafe\|Ordering::' --stats` before revisiting |

---

## 9. COVERAGE DEBT

Deferred deliberately, so each entry is a decision rather than an omission. None blocks the Trame MVP,
and epic `E9` of `ambre-roadmap.md` schedules them.

| Debt | Why it is not built |
| :--- | :--- |
| An `ambre-app` end-to-end headless boot | a real boot spawns the Logic thread, a second wasm artifact for Sync, and a wgpu surface. The `wasm-bindgen-test` harness provisions none of the three: no shared-memory page, no worker loader, no GPU adapter. A true belt needs browser automation against a served build, outside the unit-test runner |
| An end-to-end command across the canvas and DOM boundary | three cases need a real canvas and a driven browser, so they wait on the same automation as the boot above: a click travelling `CommandId` -> `LogicCommand` -> `UiEvent` -> chrome toggle plus canvas repaint, one IME composition cycle (`CHROME-007`), and rAF suspension and resumption behind a modal |
| `ambre-chrome` DOM interactions | the reactive store, the toast and the shell cell are covered. `Dialog` (modal, Escape, backdrop click, the canvas suspend and resume) and `ContextMenu` (keyboard navigation, typeahead, outside close) need richer driving. `CommandPalette`, `Sidebar` and `Toolbar` are unbuilt, and get a belt when they land |
| The GPU "how to draw" half | section 5.B: the upload and the shaders need a real device. The CPU work beside them is extracted and host-tested |
| `ambre-sync` benchmarks, and further pipeline tiers | they land when the pipeline exits placeholder |
| A real S3 round trip | nothing in the suite performs one, and the CI minio service is missing. A presigned upload against a local minio is the check to run before this reaches an environment that matters |

---

## 10. REJECTED

| Rejected | Why |
| :--- | :--- |
| A `QUALITY & TESTABILITY` section per spec | nineteen of them held the same four things with the names changed, and the two that seemed to hold a unique fact held it in three other places. A method restated per document is a method that disagrees with itself |
| A test list in a spec | it has no join to the suite and no way to fail. 82 names were cited across the corpus and 23 resolved |
| A "Definition of Done" per component | identical in nineteen files, and its only checkable half is already held by `contracts.py`: no row may lack a `Status` |
| A coverage percentage as the headline number | it counts lines, not rules. The number that matters is the **loose** column of `ambre-contract-ledger.md`: shipped code that nothing would fail on |
| Golden-image comparison as a routine mechanism | brittle under antialiasing and driver drift. Reserved for a precise critical need |
| A rendered reference to hold a rendering-quality rule | the drift a golden suffers from is what such a rule measures, so the two would fail together. A quality figure is held instead by a statistic over the rendered pixels -- the count of distinct coverage values across one edge, the measured width of a band -- which is stable across drivers and states the property directly (`ambre-spec-budgets.md sections 1 and 2`, `ambre-spec-drawing-surface.md`) |
