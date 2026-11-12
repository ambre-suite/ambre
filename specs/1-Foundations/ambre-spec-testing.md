# AMBRE -- SPEC: TESTING

**Type:** Horizontal (Foundation -- Quality)

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
* **A row reads one of four statuses:** `verified`, `failing`, `unimplemented` and
  `human-review`. What each one owes is `ambre-spec-template.md section 1.B`.
* **Code ships with its mechanism, or it does not ship.** A gap found after the code shipped reads
  `failing`: a lifted obstacle, a surviving mutant, an audit finding.

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
| A gate in `scripts/`, run by a `just` recipe in the CI `lint` job | holds what the compiler cannot see: the corpus, the prose, and a committed artifact that must not move | a rule about the tree rather than the code. Its own tests run under `just test-scripts`, because a gate that reads nothing and a gate that is satisfied look alike. A gate fails on a breach with no flag, and a generator writes only under `--write`, so a gate run by hand on a red tree goes red. `scripts/test_gates.py` holds both for each script |
| The primitive gate, `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job | every bare primitive outside a test is typed, or named by a line with its reason (`ARCH-033`). A secret is read in `ambre-crypto` and the relay's secrets module alone, and wiped by `ambre_core::Secret` alone (`ARCH-045`, `ARCH-046`, `ARCH-048`). A hand-written newtype over a primitive is named by a line with its reason (`ARCH-062`). The crate boundaries hold: the driver reads each resolved path against the part of the workspace it may reach, and the judge reads the graph `cargo metadata` resolves (`ARCH-001`, `ARCH-005`, `ARCH-006`, `ARCH-070`). The driver also reads the type of an expression, the closure a call is handed and the instance of a generic, and the judge counts the facts of the launcher's wiring over every unit (`ARCH-007`, `ARCH-010`, `BUS-004`, `SRV-170`) | always: it holds the newtype rule of `ai/ambre-rust-style.md`, which review alone loses on every change |
| `proptest` | universal rules over an unenumerated input space | pure logic and reference models: the formula engine, CRDT convergence, cache against rebuild |
| `cargo-fuzz` plus `arbitrary` | robustness against hostile input | every decoder fed by the relay or by a file. The relay is untrusted by design, so this is not a hardening extra |
| `cargo-mutants` | verifies the tests themselves -- a surviving mutant turns its row `failing` | periodically, starting with the crates whose belts were generated fastest |
| `cargo-llvm-cov` in diff mode | are the added lines exercised at all | as a CI gate, once the informative baseline exists |
| `criterion` | a time on the machine of the author, for the figure a budget row quotes | an operation carrying a numbered budget in `ambre-spec-budgets.md`. A time on a shared runner moves by tens of percent between runs, so it never fails a build |
| `callgrind`, a count of instructions with a CI ceiling | a performance regression never appears in a diff, and a count does not move between runs | every operation carrying a numbered budget in `ambre-spec-budgets.md`. The ceiling is the budget converted at the rate `scripts/instructions.py` states, and the CI `instructions` job fails an operation above it. It counts the native build: the phone figure stays a conversion |
| `insta` snapshots | read consequences instead of code; reveals unintended blast radius | deterministic structured output: `RecordingSurface` op sequences, wire encodings. **Acceptance stays human** -- `cargo insta accept` accepts blindly |
| `loom`, `miri` | interleavings under the C11 model, undefined behaviour under Stacked and Tree Borrows | **only if hand-written `unsafe` or lock-free atomics appear.** Measured 2026-08: zero `unsafe` in 17 crates, and atomics are flags and counters. Not adopted |

**A measure the CI holds to a ceiling repeats exactly.** Two runs of one build on one input give
the same count. Otherwise the ceiling fails a build on noise, and the noise hides a regression. A
random key per process moves the probes of every hash lookup, so it moves a count of instructions.
On 2026-11-09 a Trame block undo read 73 597 735 against a ceiling of 73 500 000, with no change
to the code.

* **Each source of variation is fixed in the bench build, and only there.** The bench build of
  `scripts/instructions.py` turns on the `fixed-hash-key` feature of `ambre-core`, so every table
  of `ambre_core::hash` takes foldhash's fixed key. No other build may (`ARCH-089`).
* **A bench pins the identity of each document it builds.** `with_peer` fixes the Loro peer and
  turns off the commit timestamps, which a random peer and the clock would otherwise move.
* **A ceiling that is no budget is the count plus 0.5 %**, rounded up to the next 10 000. The
  margin absorbs a change elsewhere in the code, and a regression above it fails. The budget row
  records each reading with its date. A ceiling that is a budget stays the budget.
* **A new counted operation shows three identical counts** before its ceiling lands.
* **Two counts still move, from a dependency's hasher.** `remote_batch` moves by about 0.1 %:
  a block cache of Loro keys `ahash` at random. `paragraph_shape` moves by about 0.3 %: parley and
  fontique seed hashbrown from the clock. Both ceilings are budgets, far above the count, and the
  roadmap holds the fix.

**The primitive gate reads the code through rustc.** Its driver, `crates/ambre-primitives/driver`,
stands in for rustc in each compile of the lint. It reads each item with the types rustc gives it,
after every macro expands, and never takes a site from test code.

**The gate guards against accidental misuse.** It catches a contributor who writes plain code, and
it fails closed on each form it cannot follow. It does not hold a deliberate evasion by a
committer, who can ship a build script or a dependency anyway. Review of every change holds that
threat (`ARCH-047`). Two examples:
* a function that lends a bare secret out under another name;
* a build outside the compiles of the lint (`ARCH-077`).

Test code is what a test build compiles: `cfg(test)`, a test macro, an integration test, a bench
and an example. A module name and a feature such as `test-helpers` mark none, since a release build
can compile either. A site is a field, a parameter, a return, a constant, a static, an alias or an
associated type of an impl. Its type holds a primitive, each alias resolved. Its key names the
crate, the file, the module, the item, the kind and the type as written, and no line number.

* **One mechanism, decided on 2026-11-08.** Every check that reads a type runs in the driver. That
  covers the sites, the privacy and `bool` refusals, the literals and the `Deref` targets. It
  covers the secret names, the census, the aliases, the private fields and the unwrap check too.
  The table holds the figures of the spike that decided it, read on commit `968e85c`. It holds
  those of the switch too, read on 2026-11-08 with `just primitives` and `wc -l`, tests apart.

  | Measure | syn walk | rustc driver |
  | :--- | :--- | :--- |
  | Allowlist lines the driver finds | -- | 4 178 of 4 312 at the first try. Then 4 312 of 4 316, once it read an `impl Trait` parameter, an `async fn` return and a component body. The 4 left were stale |
  | Unwrap refusals the other missed | 14 | 1, a false refusal of the syn walk |
  | Sites left ambiguous | 137 | 0 |
  | Code of the gate | 12 198 lines | 5 179 lines: the driver 2 714, the judge 2 465 |
  | Run, every workspace unit compiled again | 7.3 s to 16 s | 106 s to 110 s for the nine lint lines, `ambre-spec-budgets.md` section 7 |

  The tracks it refused are rows of section 10. The cost is the `rustc-dev` component, which the
  CI `lint` job alone installs, and one edit of the driver per deliberate bump of the nightly.

* **rustc holds the files** (`ARCH-050`). The gate reads the compile lines of the `lint` and
  `lint-wasm` recipes from the `justfile`. It runs each as `cargo check`, with its flags and its
  arguments. A line the gate cannot run as written fails closed: a shell expansion, or an argument
  that changes what the gate reads. Clippy holds the wrapper slot of the lint, so the gate drops
  the fingerprints of the workspace units first, and the dependencies stay fresh.

  The driver writes one record file per unit. A unit cargo builds without a record, through a cache
  or a wrapper, fails the gate. So does a failed compile.

  The driver records each file rustc read, a `#[path]` module and an `include!` file included. A
  `.rs` file under the root that no compile
  reads fails, test builds included. The check leaves out the `.git/`, `target/` and `vendor/` of
  the root, and the nested workspaces `fuzz/` and the driver. It refuses any other manifest that
  declares a workspace.

  Each feature a workspace manifest declares is on in a shipped build of these compiles, as cargo
  reports it. Otherwise a line of `specs/1-Foundations/ambre-features.tsv` excludes it with its
  reason. A feature no compile turns on fails, and so does a line that names no declared feature,
  or one a compile turns on. A build none of these compiles makes is a reviewer's (`ARCH-077`).

* **The primitive allowlist is `specs/1-Foundations/ambre-primitives.tsv`.** One line per site:
  the key, a category and a note. The categories are a closed set: `free-text`, `foreign-api`,
  `newtype-inner`, `trait`, `generic`, `math` and `local`. No category means "to type later",
  so a value still to type fails the gate until the change types it (`ARCH-033`).
* **The file only shrinks on its own.** A site no line names fails, and so does a line that names no
  site (`ARCH-034`). `just primitives-prune` removes those lines, and no mode adds one.
* **A new line is a hand edit, read by a reviewer** (`ARCH-035`). The gate checks the word, and
  never the meaning a line claims. A `local` line meets the four conditions of
  `ai/ambre-rust-style.md`, and its note names the reason.
* **A `newtype-inner` site is the own representation of its type** (`ARCH-036`). The driver reads
  that shape from the code. It is the field of a struct of one field, a `PhantomData` marker apart,
  or the one field of a tuple variant. It may be a parameter or a return of the codec of such a struct or
  of a fieldless enum. It may be an associated type that a generic newtype holds as its one field.
* **A `trait` site sits in an impl of a foreign trait** (`ARCH-036`), or is a closure type that
  names a foreign type. A primitive the impl writes as a trait argument, or as the type it is for,
  refutes the line: the workspace chose it.
* **Every alias reads as its target**, a foreign one included (`ARCH-044`). A type spelled `Self`
  and a default argument write no primitive. The key keeps the type as written.
* **A macro of the workspace writes sites as the source does** (`ARCH-043`), keyed at its call. A
  foreign macro writes sites from its own signatures, so the driver reads none there. A reviewer
  reads the types the source hands one (`ARCH-076`). The body a Leptos component holds is
  read as the component's own, keyed under its name.
* **A `Deref` to a primitive fails, whatever the file says** (`ARCH-037`). The driver reads the
  target rustc gives each `Deref` and `DerefMut`, whoever writes the impl. A type parameter and a
  projection fail closed.
* **A site that is not private fails three checks, whatever its category.** A site is not private
  when it is `pub`, `pub(crate)`, `pub(super)` or `pub(in path)` as written, or sits in a trait
  impl. A field is not private either on a type of `ambre-bus`, or on a type that derives
  `Serialize`, `Deserialize` or `FromRow`.
  1. A `local` line on such a site fails (`ARCH-038`).
  2. Such a field that holds a `bool` fails, unless its line is `foreign-api` (`ARCH-039`). A
     `bool` in a callable's signature is not held. The one field of a tuple variant with exactly
     one field may be a `bool`.
  3. Such a `bool` parameter fails when a call passes it a literal, unless its line is `trait`
     (`ARCH-040`).
* **The literal check reads every compile, tests included.** The driver resolves each call to its
  `fn`, so a literal reaches one parameter. A trait method reaches the trait item and its impl. A
  prop of a component reaches its parameter by name. A literal counts in the shapes `ARCH-040`
  names. A call through a value, and a literal a macro writes, stay out of reach (`ARCH-041`).
* **A tuple struct holds no field that is not private** (`NEWTYPE-001`). The driver refuses each
  one, and no line admits it.
* **A secret is read in two places and wiped in one module** (`ARCH-045`, `ARCH-046`,
  `ARCH-048`). Outside test code, the driver refuses `expose`, `expose_mut` and `clone_secret`
  outside `ambre-crypto`, `crates/ambre-core/src/secret.rs` and the relay's
  `crates/ambre-server/src/secrets/`. It refuses an item of the `zeroize` crate, or one of its
  names, outside that one file. A manifest that renames the crate fails too.
  * It refuses a sink outside the file of its one foreign call (`ARCH-049`).
  * It reads a name a path, a method call or a declaration writes, in the code each macro expands
    to. So a method of another type with a refused name fails closed.
  * An exemption keys on the file with each `..` undone, and on the file with each link followed.
    A check applies when it applies to one of the two.
  * A function that lends out the bare value under another name is a reviewer's (`ARCH-047`).
* **The unwrap check reads each body with its types** (`ARCH-073`). A refusal names its key, and a
  line of `specs/1-Foundations/ambre-unwrap.tsv` excuses a key with a count and a reason.
* **The driver links the private crates of rustc, on the pinned nightly.** It needs the
  `rustc-dev` component and no crate of its own. It builds from outside the repository, since the
  `build-std` key of `.cargo/config.toml` would link a second `std` beside rustc's own.
  `just primitives-driver` runs its format, its clippy and its fixture tests, which compile fixture
  crates through it. The judge, `ambre-primitives`, reads the records with `serde_json` for the
  messages of cargo and `toml` for the manifests.

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
| A GPU pipeline | assisted review plus a visual check | `human-review` through `/run`, on an obstacle below that says who checks it and when. Surface minimisation keeps this small: any rule that can live in pure Rust must live there |
| DOM glue | extraction first, then the browser | the decision logic on host (section 5.A), the rest in the wasm tier or in the browser tier. Never `human-review`: the browser tier reaches it |

**CRDT convergence is mandatory.** Every collaborative CRDT state ships a property test of
convergence. It generates operation sequences with `proptest`, interleaves them across two peers or
more, and asserts identical final states once every delta has been exchanged.

**An obstacle is why no mechanism reaches a rule, and it is named.** A `human-review` row
claims a gap that cannot close. That claim is a fact about the tooling, and
tooling moves. So the row's `Gap` opens on `` unheld -- `<obstacle>` -- ``, one obstacle of the
table below. A rule a mechanism holds in part is two rows, and only the half left names an obstacle.

**The five obstacles.** Read on 2026-11-08 with `just contracts`, the first two hold one row each:
the frame on the phone and the page a deployment serves. The third holds three rows that a reader
decides by meaning: `ARCH-035`, `ARCH-063` and `ARCH-076`. On 2026-11-10 it gains `INTEROP-025`
and `INTEROP-026`: whether an input fixture is a real document, and whether its licence is true.
The fourth holds five rows of the primitive gate, `ARCH-041`, `ARCH-042`, `ARCH-047`, `ARCH-067`
and `ARCH-077`. They hold what
flows at run time, what clippy scores, and the builds the gate does not run.

On 2026-11-12 the fifth obstacle holds `CANVAS-096`: a present that fails.

| Obstacle | What no mechanism reaches | Who checks, and when | Lifted by |
| :--- | :--- | :--- | :--- |
| `reference-device` | a figure taken on the reference phone. CI runs no phone and no GPU, so a frame's tessellation, upload and GPU time are timed by hand on the device | the author of a change to a frame phase, on the reference phone, before the merge; the release owner, on a busy board, before each milestone release | -- |
| `deployment` | what a deployment serves. Its hosting and its headers live outside this repository, so only loading the deployed page reads them | the operator of each deployment, in the console of its page, after each deploy that changes its hosting or its headers | -- |
| `domain-meaning` | whether a primitive is right for its site. Whether a `u32` counts rows or names one is a fact about the domain, not about the tokens, so only a reader decides it. So is which type a foreign macro takes from the source, and which it writes from its own signatures. So is what a file a test reads says, and which licence its upstream carries | the reviewer of a pull request that adds an allowlist line or an input fixture, before the merge | -- |
| `type-resolution` | what the compiles of the lint do not settle: which `fn` a call through a value reaches, which signature takes a closure, which value a function hands back, which files a build the gate does not run reads, how clippy scores a type written out at a use, and what a committer hides from the gate on purpose. The primitive gate reads each compile of the lint through rustc, with types, and nothing else | the reviewer of a pull request that changes a generic signature, a public trait or a line of `specs/1-Foundations/ambre-primitives.tsv`, before the merge | -- |
| `gpu-failure` | a failure of the GPU that a page cannot cause. wgpu over WebGL2 hands a frame a swapchain texture even after the context is lost, so no tier makes a present fail | the reviewer of a pull request that changes the present path of `crates/ambre-app/src/host`, before the merge | -- |

* **`Lifted by` names the Story that brings the mechanism, or `--`.** A row citing a lifted obstacle
  fails `contracts.py`, the day the Story is written, not the day it ships. Its gap can close, so
  it is debt: `failing`, and it lands with a Story.
* **A new obstacle is a row here first.** A row citing a name the table does not carry fails.
* **An obstacle lives while a row needs it.** A standing one no row cites fails, since it would
  excuse the next row without an argument. A lifted one goes once its Story ships.
* **The day a seam or a tool reaches an obstacle**, its Story goes in `Lifted by`. The gate then
  lists every row to requalify, so no reader has to remember them.

---

## 4. THE FOUR TIERS

| Tier | Command | Holds | Cost |
| :--- | :--- | :--- | :--- |
| **host** | `just test-native` | the default tier. Pure logic, zero external dependency. The Logic thread, the Sync pipeline, every formula and CRDT handler, and the render feed | fast |
| **db** | `just test-db` | `ambre-server` integration behind the Cargo feature `db`. `just services-up` (postgres, redis and a prepared garage), then `#[sqlx::test]` per test | medium |
| **wasm** | `just test-wasm` | every `cfg(target_arch = "wasm32")` surface: DOM and Leptos, GPU. `cargo test --target wasm32-unknown-unknown` under `wasm-bindgen-test-runner`, in headless Chrome | slow: `-Z build-std` rebuilds `std`, then Chrome starts |
| **browser** | `just test-browser` | the whole application, as a user meets it: the Trunk build served in front of a relay, driven by headless Chrome through chromedriver (`ambre-belt`, behind the Cargo feature `browser`), and by headless Firefox through geckodriver under the Cargo feature `firefox` (`just test-browser-firefox`). A canvas surface is read through what the page exposes: the live region, the status bar, the formula bar's name box, an export. A listener, a browser API, a file picker, a download, a queue the Logic thread leaves full | slowest: one Chrome and one relay per test, and the services of the db tier |

**The backing services run in containers, never on the host.** Postgres, Redis and Garage run from
the images `docker-compose.yml` pins, through `just services-up` on a workstation or a cloud
session, and as `services:` in CI. A package manager installs another version than the one the
code is written against. Redis 7.0 lacks `HEXPIRE`, and a test that fails there fails for the
environment. The pinned image is the only version a tier result
speaks for. Production deployment is out of scope of this rule.

**An image is pinned by a release and a digest.** A tag such as `8-alpine` moves to each new
release, and a registry can push a release tag again. So each image names its release and the
digest of its multi-arch index, and the CI `services:` name the images of `docker-compose.yml`
(`ARCH-087`, `ARCH-088`). Docker runs the digest. The tag tells the reader which release that is.
A bump resolves both again, the same day, in both files.

**The browser tier pauses a worker, never the product.** A test that needs a thread to stall
pauses that worker through the DevTools Protocol. The product carries no hook for a test. A browser
behaviour no driver can produce, such as a full quota, is a page script in the test. The row's
`Mechanism` note says so. A test that pauses a worker runs on Chrome alone, and reads `ignored` under
Firefox, which WebDriver gives no route to pause one.

**A composition and a frame loop are driven, not assumed.** A composition goes through the DevTools
input methods, `Input.imeSetComposition` then `Input.insertText`, which raise the composition events
through Chrome's own pipeline. A page script installed before boot counts the WebGL draw calls,
and that count reads the frame loop back. A paused loop still takes its animation frames, so a count
of those reads nothing.

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

`just test-wasm` covers `ambre-trame`, `ambre-chrome`, `ambre-toile`, `ambre-logic`, `ambre-prose`,
`ambre-murmure` and `ambre-app`.

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
| `ambre-app` | `switchboard`, the decisions of the switchboard: the view fan-out, the header status, the busy notice, the prompt of a fetched key, the request of a trusted key, the requests and notices of a service answer, the cadence of the perf read-out (`ARCH-072`) | the closure that applies them, and the glue modules the frame loop drives: `chrome_mirror`, `murmure_mirror`, `route_binding`, `tab_title`, `persist` and `anchor`; `route::parse`, `host::base64_*` and `FrameHost::tick_fps`, which the crate's other pure helpers still run as inline `#[wasm_bindgen_test]` |
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
  TCP connection, a CORS refusal. That stays with the browser tier (section 4).

---

## 6. REALIZATION PER CRATE

One row per crate: what runs, what stands in for the world, and the belt an increment lands with.

| Crate | host | wasm | Seams | Belt |
| :--- | :--- | :--- | :--- | :--- |
| `ambre-core` | yes | -- | -- | types, plus the `.ambre` container: every header failure (truncated, bad magic, `UnsupportedVersion` in both directions, `WrongDocType`), byte soup without a panic, and the migration walk driven by synthetic steps, because no second version exists to exercise it. The regression belt over real files is the fixture corpus (`ambre-spec-fixtures.md`). `Secret` adds its own: a debug form with no byte and no length, one wipe on drop seen through a spy, an equality that reads every byte, and `compile_fail` doctests for a derived `Clone`, `Serialize` or `PartialEq` on a type that holds one |
| `ambre-crypto` | yes (`proptest`, `criterion`) | -- | -- | round trip, tampering, zeroize, **plus human review**. The file key adds its own: the derivation is deterministic per salt, a `FileKey` and a user key from the same inputs differ, and a sealed round trip fails closed on a wrong passphrase (`DOCFILE-016`) |
| `ambre-protocol` | yes | -- | -- | postcard wire round trip |
| `ambre-bus` | yes (`tests/host.rs`) | -- | `CommandSink`, the trait a controller sends through | Logic/Sync envelope postcard round trips, with the `id` echo preserved. The round trip is a sample rather than an enumeration: 5 of 12 `SyncCommand` variants and 4 of 19 `LogicEvent` variants (`BUS-003`) |
| `ambre-logic` | yes | -- | mock queues on both sides | typed dispatch: forward, field preservation, drop on an unknown document, the role gate on a viewer and on an embedded child, the service route and its correlated failure |
| `ambre-sync` | yes | -- | `MockWsTransport`, an in-memory socket; `MockBlindRelay`, an in-process relay; `LogicSink::queue` | four, because this is the boundary the product rests on. **Round trip** per message type. **Tampering**: one mutated byte, a wrong key, a wrong `document_id`, a wrong signature -- each fails closed. **Ordering**: an update ahead of its snapshot is quarantined, released in clock order, and the quarantine is bounded. **Replay**: a non-increasing counter is dropped. The two sealed-file handlers are the exception to the boundary framing -- they reach no relay, and their belt is a round trip at the real work factor plus one failure shape for a wrong passphrase and a tampered file |
| `ambre-canvas` | yes | GPU not unit-tested, by design | `RecordingSurface` | `TextCache` hit and LRU eviction, `GlyphAtlas` eviction, the dirty flag skipping the batch build, font fallback on an absent codepoint, and the compiled icon blob covering every SVG of `assets/icons/` with a non-zero vertex count per icon |
| `ambre-commands` | yes | -- | -- | registry lookup, keybinding resolution, derived `CommandState` |
| `ambre-trame` | yes (`proptest`) | yes (`tests/chrome_ui.rs`) | `MockSyncHandle`, `TrameState::with_peer`, `RecordingSurface`, a `TrameView` on a detached pipe and a test clock | **CRDT convergence** over generated interleavings. **Formula properties**: totality, exactness, order independence over generated trees. **Cache against rebuild** on the `SheetIndex`. A **geometry round trip**: drawn rect against hit test, under zoom and variable metrics. A `RecordingSurface` assertion for anything painted. **Per feature**: a view feature lands with a positive case, a no-op guard, a convergence case and a proof that it changes no data; a mutation feature lands with the permutation assertion, a header-pin case, a no-delta case and a references-follow case |
| `ambre-chrome` | yes (`key_from_dom`, `place_menu`, `vet`) | yes (`tests/chrome_ui.rs`) | section 5.A | the reactive store over real signals, the toast ARIA live region, the shell canvas cell and its toolbar and sidebar slots, the menu flip on viewport overflow, the modal, the floating menu driven by keys and presses, and the palette driven by keys |
| `ambre-app` | yes (`switchboard`) | yes (inline) | -- | the switchboard's decisions on the host: the view fan-out keeps the order of a refused event and drops nothing in silence, for the main view and for each child, a fetched key reaches the prompt with its purpose, and the busy notice reads the edge alone. `route` round trip and malformed fallback, the base64 codec, the FPS sliding window. A re-render is read after `leptos::task::tick`, and a width by mounting in a frame of that width, since a width query reads the frame's viewport |
| `ambre-server` | yes | -- (db tier) | `#[sqlx::test]`, a local Redis | ACL per role **and per membership**, Ed25519 accept and refuse, the Redis Lua clock cases (valid, mismatch, first update, two concurrent clients), the rank clamp on both the granted and the target role, the size cap on both durable paths, and a postcard round trip per message variant -- the document plane only: the four channel variants and the share notice have none |
| `ambre-toile` | yes | yes (`tests/chrome_ui.rs`, `tests/chrome_effects.rs`) | `ToileState::with_peer`, `RecordingSurface` | CRDT convergence, and the spatial index against a full rebuild |
| `ambre-draw`, `ambre-pdf` | tooling | -- | they **are** the seam | `RecordingSurface` is exercised by every consumer |
| `ambre-opc` | yes | -- | -- | the package layer the office codecs share: a written package reads back with its types and its relationships, the same parts write the same bytes, a part name never climbs out of the package, a dangling relationship is refused, and one test per bound: the part count, one part, and the parts together. An OpenDocument package reads back under its type, and a package of another type or of none is refused |
| `ambre-pptx` | yes (`proptest`) | -- | `ambre_opc::Package`, through which each test opens the written file back whole | the file opened back: every relationship reaches a typed part, every XML part parses, and any text reads back as it left |
| `ambre-prose` | yes (`proptest`, `criterion`) | yes (`tests/chrome_panel.rs`, `tests/chrome_ui.rs`) | `ProseState::with_peer`, `RecordingSurface` | CRDT convergence of text and annotations, placement properties over generated documents, and a `RecordingSurface` assertion for anything painted. The comment panel, the suggestion popover, the format bar, the slash menu and the ruler are mounted in Chrome |
| `ambre-sfu` | yes | -- | an in-memory link between two str0m connections | the DTLS handshake between a participant and the SFU over the RustCrypto backend, and its refusal of a wrong fingerprint (`CALL-031`). The lockfile gate holds what it links (`CALL-013`) |
| `ambre-belt` | -- (browser tier) | -- | a static server in front of a relay process; the DevTools Protocol, to pause a worker | the browser tier itself: one file per surface, each test a user gesture and the answer read off the page. A peer is a second browser signed in as a second user. Trame and Prose are typed into end to end, and each converges across two browsers whose edits cross; Toile carries a board between two; two Murmure users share a channel, its unread count, a thread reply and a direct message |
| `ambre-primitives` | yes | -- | fixture crates the driver compiles in a temporary directory; record files held in memory | the primitive gate itself. The driver tests compile fixture crates through it: every kind of site read with its facts, test code never read, a macro of the workspace read and a foreign one not, each literal resolved, each refusal of a `Deref`, a secret name, a sink and a public tuple field, the census, the aliases, the unwrap sites, each file rustc reads, and each crate boundary crossed through an alias, a re-export or a macro. The judge tests read records: the allowlist failing on a missing site, a stale line, a word outside the set and a refuted claim, the lint lines, the features, a file no compile reads, a wire type redeclared, and each manifest boundary over a fixture graph of `cargo metadata` |
| `ambre-murmure` | yes (`proptest`, `criterion`) | yes (`tests/chrome_relay.rs`) | a `ChannelState` peer forked from another, a `MurmureRelay` fed by hand | CRDT convergence of a channel across its `meta`, segment and thread documents over generated interleavings, a forged delta refused by the authorship check, and a false final snapshot caught by the second client. Eviction is the shared residency's belt, and the decoded pictures hold their own cap. The browser tier drives two users through the relay |

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
| `lint` | `cargo fmt --check`, `cargo clippy --all-targets -D warnings` on host with every feature on, the five corpus scripts, the driver of the primitive gate and the gate itself. It alone installs `rustc-dev` |
| `test-native` | the host tier, plus `ambre-server` on the native target, then `just fixtures-check`: the corpus regenerated against its committed bytes and manifests |
| `test-db` | postgres and redis as services, garage started by `just garage-up`, with `--features db` |
| `test-wasm` | `just lint-wasm`, clippy under `-D warnings` on the wasm target, then `just test-wasm`: `cargo test` on the wasm target under `wasm-bindgen-test-runner`, in headless Chrome |
| `test-browser` | the db-tier services, the Trunk build and the relay, then `just test-browser` and `just test-browser-firefox`, a matrix over two engines. Firefox runs under `xvfb-run`, since headless Firefox makes its WebGL2 context through an X display. Firefox also needs an audio server: without one, its audio context stays `suspended`, and the meter and the gain of a call read zeros. The job starts `pulseaudio` with a null sink, which plays nothing |
| `deny` | `cargo deny`: advisories, licences, sources, and the capability bans |
| `coverage` | `cargo-llvm-cov` over the host tier, non-blocking. An `lcov` baseline as an artifact |
| `mutants` | `cargo-mutants` over the scope of `.cargo/mutants.toml` on the host tier, eight shards, on the default branch alone, non-blocking. Each shard uploads its report |
| `mutants-diff` | `cargo-mutants --in-diff` on the lines a pull request changes in that scope, blocking (`ARCH-069`) |

The wasm `-A warnings` rustflag is scoped to `[target.wasm32-unknown-unknown]`, so host clippy keeps
every warning. `lint-wasm` sets its own `RUSTFLAGS`, which replace that rustflag. Every native job
installs `libfontconfig1-dev`: the font stack links system fontconfig.

---

## 8. MECHANISM GAPS

What section 2 prescribes, against what is wired. Measured 2026-08. Each absence is a decision rather
than an omission. Epic `E9` of `ambre-roadmap.md` schedules them, and the milestone of each story says
whether it blocks the current MVP.

| Mechanism | State | Gap |
| :--- | :--- | :--- |
| Type-level invariants | **the strongest area**: the id newtype macro, `PublicKey`, `Payload<Ciphertext>`, and `Secret` under every secret type | not the mechanism, the *statement*: outside `Secret` (`ZK-019`, `ZK-020`), nothing says which invariants these types hold, so the guarantee is invisible |
| `#![forbid(unsafe_code)]` | **wired** | `unsafe_code = "forbid"` in the workspace lint table, so all 18 crates refuse an `unsafe` block |
| `[workspace.lints]` | wired | every crate inherits it (`ARCH-032`). It denies `missing_errors_doc`, `missing_panics_doc` and the panic lints (`ARCH-028`); no `pedantic`. Clippy reads the wasm-only code in the `test-wasm` job |
| `cargo-deny` | **wired** | advisories, licences, sources, and the capability bans that mechanise five ZK rows. Negative-tested |
| `proptest` | wired | `ambre-crypto`, the Trame formula engine, convergence on Trame, Toile and Murmure, and the Toile index against rebuild |
| `cargo-fuzz` | wired | seven targets in `fuzz/`: the wire decode, the `.ambre` container, the formula parser, the Markdown reader, the sealed open path, and the Trame and Toile file readers, one minute each in the CI `fuzz` job. The sealed target opens under a fixed key, so a key-holding peer's plaintext reaches the decompression |
| `cargo-mutants` | wired, host tier | `ambre-sync`, `ambre-crypto` and the Murmure channel module, measured 2026-11-05: 2862 mutants, and 183 survivors left after triage. A second reading the same day covered `ambre-sync` and the fold, relocation and thread modules, through `cargo mutants -F` in four slice shards: 1904 mutants, 162 survivors and 1 timeout. Each survivor then got a test seen killing it in a targeted rerun, or an exclusion with its reason. No full run has measured the scope since. The wasm and browser tiers run per mutation nowhere |
| `cargo-llvm-cov` | wired, non-blocking, host tier alone | an `lcov` baseline only. No diff-mode gate on added lines. The wasm tier has none: `-C instrument-coverage` needs `-Zno-profiler-runtime` on wasm32, a flag of the nightly that no line of the CI sets |
| `criterion` | benches in `ambre-crypto`, `ambre-toile` and `ambre-murmure` | a desktop figure only. **The mobile measurement the targets are written against has never been taken** |
| `callgrind` | wired, five budgets | the CI `instructions` job holds the Toile body commit, the content walk of a busy Toile board, the Prose text layout of a paragraph, the Trame re-anchor walk and the Trame remote batch under their ceilings, and the bound the Toile diff cap allows. Every other budget row has no ceiling yet |
| `insta` | absent | candidates: `RecordingSurface` op sequences, wire encodings, and the `KrillaSurface` primitives |
| `loom`, `miri` | **not adopted, by decision** | zero `unsafe`, and atomics are flags and counters. Re-check with `rg 'unsafe\|Ordering::' --stats` before revisiting |

---

## 9. COVERAGE DEBT

Deferred deliberately, so each entry is a decision rather than an omission. None blocks the Trame MVP,
and epic `E9` of `ambre-roadmap.md` schedules them.

| Debt | Why it is not built |
| :--- | :--- |
| One IME composition in a Toile text element | an `Escape` sent just after a commit closes the editor without the committed text, so a browser test of it fails one run in a few. A Prose page and a Trame cell compose in the browser tier |
| A Prose comment between two browsers | no gesture opens a thread, and a thread read from a file stays in a document no peer holds: the browser tier posts a reply in one browser alone |
| Murmure in the browser tier | no interface creates a space or a channel, so no user gesture reaches one, and the belt drives no Murmure path |
| The wasm tier on Firefox | Trame, the chrome, Toile, Prose and Logic pass under geckodriver. Eleven `ambre-app` tests build synthetic clipboard, pointer and layout events that Firefox handles differently, so the tier runs in Chrome alone |
| The GPU "how to draw" half | section 5.B: the upload and the shaders need a real device. The CPU work beside them is extracted and host-tested |
| `ambre-sync` benchmarks, and further pipeline tiers | they land when the pipeline exits placeholder |

---

## 10. REJECTED

| Rejected | Why |
| :--- | :--- |
| A wider margin over a count that moves from run to run | it hides a real regression of the size of the margin, and the noise still sets the margin. A fixed key in the bench build removes the noise, so a margin of 0.5 % holds (`ARCH-089`) |
| The primitive gate on `syn`, kept by decision | the walk resolved names and types again: glob imports, re-exports, aliases, a guard on files rustc reads. Each approximation became a story, and 137 unwrap sites stayed ambiguous. The driver reads the types rustc gives, and found 14 refusals the walk missed |
| Clippy's `disallowed-methods` for the secret checks | it holds the secret names alone, and adds a second mechanism beside the driver for 913 lines. Twenty files would carry an expectation each, several of them for test modules |
| The crates of rust-analyzer for the gate | one load reads one target, so the two wasm-only newtypes went unread. Version 0.0.349 built only once `salsa` was pinned below a patch release that broke it |
| Dylint for the driver | it adds `cargo-dylint` and a `clippy_utils` pinned to one nightly, and the bare driver needs neither |
| One `#[expect]` per allowlist line | 4 312 attributes in the code, and a reviewer reads a line of the allowlist as one file |
| A `QUALITY & TESTABILITY` section per spec | nineteen of them held the same four things with the names changed, and the two that seemed to hold a unique fact held it in three other places. A method restated per document is a method that disagrees with itself |
| A test list in a spec | it has no join to the suite and no way to fail. 82 names were cited across the corpus and 23 resolved |
| A "Definition of Done" per component | identical in nineteen files, and its only checkable half is already held by `contracts.py`: no row may lack a `Status` |
| A coverage percentage as the headline number | it counts lines, not rules. The number that matters is the **loose** column of `ambre-contract-ledger.md`: shipped code that nothing would fail on |
| Golden-image comparison as a routine mechanism | brittle under antialiasing and driver drift. Reserved for a precise critical need |
| A rendered reference to hold a rendering-quality rule | the drift a golden suffers from is what such a rule measures, so the two would fail together. A quality figure is held instead by a statistic over the rendered pixels -- the count of distinct coverage values across one edge, the measured width of a band -- which is stable across drivers and states the property directly (`ambre-spec-budgets.md sections 1 and 2`, `ambre-spec-drawing-surface.md`) |
