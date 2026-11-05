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
| The primitive gate, `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job | every bare primitive outside a test is typed, or named by a line with its reason (`ARCH-033`). A secret is read in `ambre-crypto` and the relay's secrets module alone, and wiped by `ambre_core::Secret` alone (`ARCH-045`, `ARCH-046`, `ARCH-048`). A hand-written newtype over a primitive is named by a line with its reason (`ARCH-062`) | always: it holds the newtype rule of `ai/ambre-rust-style.md`, which review alone loses on every change |
| `proptest` | universal rules over an unenumerated input space | pure logic and reference models: the formula engine, CRDT convergence, cache against rebuild |
| `cargo-fuzz` plus `arbitrary` | robustness against hostile input | every decoder fed by the relay or by a file. The relay is untrusted by design, so this is not a hardening extra |
| `cargo-mutants` | verifies the tests themselves -- a surviving mutant is unverified code | periodically, starting with the crates whose belts were generated fastest |
| `cargo-llvm-cov` in diff mode | are the added lines exercised at all | as a CI gate, once the informative baseline exists |
| `criterion` | a time on the machine of the author, for the figure a budget row quotes | an operation carrying a numbered budget in `ambre-spec-budgets.md`. A time on a shared runner moves by tens of percent between runs, so it never fails a build |
| `callgrind`, a count of instructions with a CI ceiling | a performance regression never appears in a diff, and a count does not move between runs | every operation carrying a numbered budget in `ambre-spec-budgets.md`. The ceiling is the budget converted at the rate `scripts/instructions.py` states, and the CI `instructions` job fails an operation above it. It counts the native build: the phone figure stays a conversion |
| `insta` snapshots | read consequences instead of code; reveals unintended blast radius | deterministic structured output: `RecordingSurface` op sequences, wire encodings. **Acceptance stays human** -- `cargo insta accept` accepts blindly |
| `loom`, `miri` | interleavings under the C11 model, undefined behaviour under Stacked and Tree Borrows | **only if hand-written `unsafe` or lock-free atomics appear.** Measured 2026-08: zero `unsafe` in 17 crates, and atomics are flags and counters. Not adopted |

**The primitive gate reads the items rustc reads.** It parses each crate from its roots without
expanding a macro, and never takes a site from test code.

**The gate guards against accidental misuse.** It catches a contributor who writes plain code, and
it fails closed on each form it cannot follow. It does not hold a deliberate evasion by a
committer, who can ship a build script or a dependency anyway. Review of every change holds that
threat (`ARCH-047`). Five examples:
* a macro trick;
* build configuration, an environment or a wrapper outside the forms the gate checks;
* a recipe line that shell quoting or a line continuation hides from the literal tripwire;
* a `${{ vars.* }}` in the CI workflow;
* a proc or a foreign macro.
 Test code is what `cfg(test)`, a
`cfg(all(test, ..))` or a test macro marks. A module name and a feature such as `test-helpers`
mark none, since a release build can compile either. A site is a field, a parameter, a
return, a constant, a static or an alias whose type holds a primitive. Its key names the crate, the
file, the module, the item, the kind and the type, and no line number.

* **rustc holds the files the walk reads** (`ARCH-033`). The gate reads the compile lines of the
  `lint` and `lint-wasm` recipes from the `justfile`, and runs each as written. cargo then deems
  fresh each unit the lint built, so the gate compiles nothing the lint compiled. It reads the
  dep-info of each unit of the workspace, where clippy lists its configuration as data.

  A line the gate cannot run as `just` runs it fails closed. That is a prefix, a shell expansion,
  or an argument that changes what the gate reads. A file that a shipped build reads fails, whatever its
  name, unless the walk reads it or a walked file names it as data. Data is the string literal of
  an `include_str!` or an `include_bytes!`, or the directory a `sqlx::migrate!` names. A bare
  `migrate!` counts only where a `use` imports `sqlx::migrate`.

  An `include!` file, a misread `#[path]` and a data path a macro builds each land there. The
  dep-info does not tell a module from data, so a `mod` a macro writes could compile a data file
  unseen. The scan refuses that door: a `mod` keyword a macro holds fails, raw or not, in a
  `macro_rules!` body, a macro call or an attribute. An inline `mod name { .. }` passes in one
  place: the literal tokens of a `macro_rules!` transcriber, outside a call the transcriber
  writes. Its child `mod` is read the same way. A macro can reorder the tokens of a call, so an
  inline module there fails.

  A `#![feature]` in a crate root fails, unless `FEATURE_ROOTS` names the root.

  A lever of the build configuration fails too: a `-Z` flag, `crate-attr` or `feature(`. Such a
  lever enables syntax the pinned syn may not parse. The check reads five sources:
  1. each `.cargo/config.toml` and `.cargo/config` under the root, above it and in the cargo
     home. It reads each `rustflags` and `rustdocflags` value, and each key of an `[unstable]`
     table. `UNSTABLE` names one key with its exact value: `build-std` of the root config, for
     the threaded wasm build. A `rustc`, `rustc-wrapper` or `rustc-workspace-wrapper` key fails
     at any depth. So do an `include` key and each `[env]` entry, since `CONFIG_ENV` names none;
  2. each `Cargo.toml`: a `cargo-features` key, and a `rustflags` or `rustdocflags` key. The scan
     reads `sqlx::migrate!` by its name, so another crate under that name fails. That is a
     rename, a `path` or `git` source, and a `[patch]` or `[replace]` of `sqlx`;
  3. the environment of the gate, before any compile. A flag variable such as `RUSTFLAGS` or
     `CARGO_ENCODED_RUSTFLAGS` fails on a lever. Any `CARGO_UNSTABLE_*` variable fails, and so
     does any variable that names `rustc` or a wrapper. CI sets none of them;
  4. the `justfile`, each line that is no comment;
  5. `.github/workflows/ci.yml`, each line that is no comment.

  Sources 4 and 5 are a literal tripwire. Shell quoting, a line continuation and a
  `${{ vars.* }}` pass it. A config or a manifest that does not parse as TOML fails. A flag set
  elsewhere is a reviewer's (`ARCH-050`): in the environment of another job, or in a `--config`
  argument the tripwire does not see.

  A data file that parses as Rust items fails too, as defence in depth. syn and rustc parse
  apart, so this guard is best effort. A refusal of what syn cannot parse would refuse the real
  shaders and style sheets, which lex as Rust tokens and do not parse.

  A test build may read any file. A `.rs` file under the workspace root that no compile reads
  fails, whatever its name. The check leaves out the `.git/` and `target/` of the root. It leaves
  out the nested workspace `fuzz/` that `NESTED_WORKSPACES` names too, and a test holds that its
  manifest declares one. Any other manifest with a top-level `workspace` key in its TOML fails. The
  listing follows no link to a directory.

  Each feature a workspace manifest declares is on in a shipped build of these compiles, as cargo
  reports it. Otherwise a line of `specs/1-Foundations/ambre-features.tsv` excludes it with its
  reason. A feature no compile turns on fails. So does a line that names no declared feature, or one a
  compile turns on.

  A failed compile and a lint the line denies fail closed. So do a missing dep-info file, a unit
  with no test flag or no feature list, and a dep-info line the parser cannot read whole.
  `just primitives-files` runs this half alone. `just primitives` runs the strict unwrap report on
  the same scan. A build none of these compiles makes, under another target, `cfg`, feature set or
  profile, is a reviewer's (`ARCH-050`).

* **The primitive allowlist is `specs/1-Foundations/ambre-primitives.tsv`.** One line per site:
  the key, a category and a note. The categories are a closed set: `free-text`, `foreign-api`,
  `newtype-inner`, `trait`, `generic`, `math` and `local`. No category means "to type later",
  so a value still to type fails the gate until the change types it (`ARCH-033`).
* **The file only shrinks on its own.** A site no line names fails, and so does a line that names no
  site (`ARCH-034`). `just primitives-prune` removes those lines, and no mode adds one.
* **A new line is a hand edit, read by a reviewer** (`ARCH-035`). The gate checks the word, and
  never the meaning a line claims. A `local` line meets the four conditions of
  `ai/ambre-rust-style.md`, and its note names the reason.
* **A `newtype-inner` site is the own representation of its type** (`ARCH-036`). The gate checks
  that shape from the site. It sits in the impl of a struct with one field, a `PhantomData`
  marker apart, or of a fieldless enum. It may instead be the one field of a tuple variant. The
  orphan rule may put that impl in the crate of a workspace trait. Its reader still picks the one
  true category of a spelling (`ARCH-035`).
* **A `trait` site sits in an impl of a foreign trait** (`ARCH-036`), or is a closure type that
  names a foreign type. A primitive the impl writes as a trait argument, or as the type it is for,
  refutes the line: the workspace chose it. A `use ... as` rename of a trait is undone before the
  lookup.
* **An alias of a module reads as its target.** Each `type` alias a module of the workspace
  declares resolves in the module that names it, across crates, before any check reads the type.
  The key keeps the type as written.
  * The path is placed through child modules, `use` items, `use ... as` and `extern crate`
    renames, and globs. A name a `use` imports that is no alias shadows one. So do a generic
    parameter of an enclosing item and an item an enclosing block declares.
  * An argument resolves where the use writes it, and takes the place of the next parameter of its
    kind. A parameter with no argument takes its default, and the target resolves in the module of
    the alias.
  * A `use ... as` that renames a primitive of the toolchain, through any chain of imports, reads
    as that primitive.
  * A path that ends in an alias name and cannot be placed fails the scan, and so does a glob `use`
    the gate cannot place. So do a chain past 32 aliases and an expansion past 4096.
  * The gate reads aliases and `use` items by module only. So a site that names an alias a block
    declares fails the scan. A path that starts with a name a block `use` or `extern crate` brings
    in fails when that item renames. It fails too when the item imports an alias, a rename or a
    primitive, or when the path ends in an alias name.
  * An alias of a foreign crate reads as itself (`ARCH-044`), and the gate reads no site a macro
    writes (`ARCH-043`).
* **A `Deref` to a primitive fails, whatever the file says** (`ARCH-037`). The gate reads the
  `Target` of each `Deref` impl, and a `DerefMut` takes the target of the `Deref` of its type in
  its module. A reference, a slice, an array and a `Vec`, `Box`, `Rc`, `Arc` or `Cow` expose their
  element. A generic target fails closed, a bound that names a primitive included, since which
  instance makes it a primitive is rustc's to resolve. A target the gate cannot read fails too.

  The impls nested in a constant, a static or an enum discriminant, and those a `macro_rules!`
  body writes, are read. A body that writes `impl ... Deref` and does not parse fails. So does
  a call of any other macro whose tokens write one, since the gate expands no macro.
* **A site that is not private fails three checks, whatever its category.** A site is not private
  when it is `pub`, `pub(crate)`, `pub(super)` or `pub(in path)`, or sits in a trait impl. A field
  is not private either on a type of `ambre-bus`, or on a type that derives `Serialize`,
  `Deserialize` or `FromRow`.
  1. A `local` line on such a site fails (`ARCH-038`).
  2. Such a field that holds a `bool` fails, unless its line is `foreign-api` (`ARCH-039`). A
     `bool` in a callable's signature is not held. The one field of a tuple variant with exactly
     one field may be a `bool`.
  3. Such a `bool` parameter fails when a call passes it a literal, unless its line is `trait`
     (`ARCH-040`).
* **The literal check reads every file of a crate, tests included.** A call matches a parameter by
  the name of its `fn` and by its position, and a `view!` prop matches a component parameter by
  name. A call in a crate that reaches the site's crate matches a `pub` site too. A crate reaches
  its dependencies, a renamed one included, and the crates they re-export. A call that matches two
  sites refutes both, since a name cannot tell them apart.

  A literal counts bare, in parentheses, or in a block or an `unsafe` block of one expression. It
  counts behind `&` or `!`, under `Some`, and in a tuple, an array or a `vec!`. It counts in a
  branch of an `if` and in an arm of a `match`. A `use ... as` rename holds in its own module,
  and a `pub use ... as` names the `fn` to every crate that reaches it. A call through a value,
  a literal in any other shape, and code a macro writes, stay out of reach (`ARCH-041`).
* **A secret is read in two places and wiped in one module** (`ARCH-045`, `ARCH-046`,
  `ARCH-048`). Outside test code, the gate refuses `expose`, `expose_mut` and `clone_secret`
  outside `ambre-crypto`, `crates/ambre-core/src/secret.rs` and the relay's
  `crates/ambre-server/src/secrets/`. It refuses a name of the `zeroize` crate outside that one
  file, and a manifest that renames the crate. The two other places that read get no wipe.
  * It refuses a sink outside the file of its one foreign call (`ARCH-049`). It refuses the name
    `include` in each file it reads and in each form the source writes, since it reads no file
    an `include!` names. A foreign macro that splices a file under another name is a reviewer's
    (`ARCH-047`).
  * It reads the files the walk reaches, and the dep-info check holds them to the files rustc
    reads (`ARCH-033`). A `#[path]` the walk cannot follow fails the scan: one inside a
    `cfg_attr`, one that is no string literal, and one on an inline module. So does one whose
    file lies outside the crate, is not `.rs` or lies under `target/`.
  * An exemption keys on the file the source names with each `..` undone, and on the file the disk
    holds with each link followed. A check applies when it applies to one of the two. So neither a
    `..` nor a link moves a file into the relay's `src/secrets/` (`ARCH-048`).
  * It reads names, not types. A method call, a path, a `use` item, a signature, an attribute and
    every token of a macro count. So a method of another type with a refused name fails closed.
  * A hand-written `Display`, `Clone` or `Serialize` of a type that holds a `Secret` reads the
    value through `expose`, and so does a `serialize_with` function. Each is refused outside the
    places that read. A derive of one of them does not compile, since `Secret` implements none.
  * A type that holds a newtype with a serde form of its own derives it with no refused name. A
    reviewer reads that door (`ARCH-047`). So also a function that lends out the bare value under
    another name, and code a macro or a derive writes.
  * The check holds a name the source writes, in a crate root or in a module a `mod` item
    declares (`ARCH-045`). A name a macro builds, and a file a deliberate evasion hides from the
    walk, are a reviewer's (`ARCH-047`).
* **It reads with `syn` and `quote`, host only.** `syn` parses Rust as rustc does, which a regex
  over the source cannot. `quote` prints a parsed type back as text for the key. `proc-macro2`,
  already under both, gives the line of a `bool` literal through its `span-locations` feature.
  `serde_json` reads the messages of each compile, which name the dep-info file of each unit.

  `toml` parses each manifest and each cargo config as cargo reads them. The lockfile already
  holds it under `leptos`. The tree walk uses `std` rather than a crate.

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
| DOM glue | extraction first, then the browser | the decision logic on host (section 5.A), the rest in the wasm tier or in the browser tier. Never `qa-manual`: the browser tier reaches it |

**CRDT convergence is mandatory.** Every collaborative CRDT state ships a property test of
convergence. It generates operation sequences with `proptest`, interleaves them across two peers or
more, and asserts identical final states once every delta has been exchanged.

**An obstacle is why no mechanism reaches a rule, and it is named.** A `qa-manual` or a
`human-review` row claims a gap that cannot close. That claim is a fact about the tooling, and
tooling moves. So the row's `Gap` opens on `` unheld -- `<obstacle>` -- ``, one obstacle of the
table below. A rule a mechanism holds in part is two rows, and only the half left names an obstacle.

**The four obstacles.** The first three hold one row each today: the frame on the phone, the page a
deployment serves, and what an allowlist line means. The fourth holds seven rows of the primitive
gate, `ARCH-041` to `ARCH-044`, `ARCH-047`, `ARCH-050` and `ARCH-067`: what rustc and clippy resolve
and syn does not.

| Obstacle | What no mechanism reaches | Lifted by |
| :--- | :--- | :--- |
| `reference-device` | a figure taken on the reference phone. CI runs no phone and no GPU, so a frame's tessellation, upload and GPU time are timed by hand on the device | -- |
| `deployment` | what a deployment serves. Its hosting and its headers live outside this repository, so only loading the deployed page reads them | -- |
| `domain-meaning` | whether a primitive is right for its site. Whether a `u32` counts rows or names one is a fact about the domain, not about the tokens, so only a reader decides it | -- |
| `type-resolution` | what rustc resolves and the tokens do not show: which `fn` a call through a value reaches, which signature takes a closure, what a macro writes, what an alias of a foreign crate stands for, which value a function hands back, which files a build the gate does not run reads, how clippy scores a type written out at a use, and what a committer hides from the gate on purpose. The primitive gate parses with `syn`, expands nothing and reads no foreign source. It reads what rustc read in its own compiles alone | -- |

* **`Lifted by` names the Story that brings the mechanism, or `--`.** A row citing a lifted obstacle
  fails `contracts.py`, the day the Story is written, not the day it ships. Its gap can close, so
  it is debt: `untested`, and it lands with a Story.
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
| **browser** | `just test-browser` | the whole application, as a user meets it: the Trunk build served in front of a relay, driven by headless Chrome through chromedriver (`ambre-belt`, behind the Cargo feature `browser`). A listener, a browser API, a file picker, a download, a queue the Logic thread leaves full | slowest: one Chrome and one relay per test, and the services of the db tier |

**The browser tier pauses a worker, never the product.** A test that needs a thread to stall
pauses that worker through the DevTools Protocol. The product carries no hook for a test. A browser
behaviour no driver can produce, such as a full quota, is a page script in the test. The row's
`Mechanism` note says so.

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
| `ambre-chrome` | yes (`key_from_dom`, `place_menu`, `vet`) | yes (`tests/chrome_ui.rs`) | section 5.A | the reactive store over real signals, the toast ARIA live region, the shell canvas cell, and the menu flip on viewport overflow |
| `ambre-app` | -- | yes (inline) | -- | `route` round trip and malformed fallback, the base64 codec, the FPS sliding window. A re-render is read after `leptos::task::tick`, and a width by mounting in a frame of that width, since a width query reads the frame's viewport |
| `ambre-server` | yes | -- (db tier) | `#[sqlx::test]`, a local Redis | ACL per role **and per membership**, Ed25519 accept and refuse, the Redis Lua clock cases (valid, mismatch, first update, two concurrent clients), the rank clamp on both the granted and the target role, the size cap on both durable paths, and a postcard round trip per message variant -- the document plane only: the four channel variants and the share notice have none |
| `ambre-toile` | yes | yes (`tests/chrome_ui.rs`, `tests/chrome_effects.rs`) | `ToileState::with_peer`, `RecordingSurface` | CRDT convergence, and the spatial index against a full rebuild |
| `ambre-draw`, `ambre-pdf` | tooling | -- | they **are** the seam | `RecordingSurface` is exercised by every consumer |
| `ambre-prose` | yes (`proptest`, `criterion`) | yes (`tests/chrome_panel.rs`) | `ProseState::with_peer`, `RecordingSurface` | CRDT convergence of text and annotations, placement properties over generated documents, and a `RecordingSurface` assertion for anything painted. The comment panel is mounted in Chrome |
| `ambre-belt` | -- (browser tier) | -- | a static server in front of a relay process; the DevTools Protocol, to pause a worker | the browser tier itself: one file per surface, each test a user gesture and the answer read off the page. A peer is a second browser signed in as a second user |
| `ambre-primitives` | yes | -- | `Tree`, a source tree held in memory | the primitive gate itself: every kind of site read, a module resolved where rustc finds it, test code and macro output never read, a file rustc reads and the walk misses failing the gate, and a file no compile reads, the allowlist failing on a missing site, a stale line and a word outside the set, and a secret read or wiped outside the code that may |
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
| `lint` | `cargo fmt --check`, `cargo clippy --all-targets -D warnings` on host with every feature on, the five corpus scripts, and the primitive gate |
| `test-native` | the host tier, plus `ambre-server` on the native target |
| `test-db` | postgres and redis as services, garage started by `just garage-up`, with `--features db` |
| `test-wasm` | `just lint-wasm`, clippy under `-D warnings` on the wasm target, then `just test-wasm`: `cargo test` on the wasm target under `wasm-bindgen-test-runner`, in headless Chrome |
| `test-browser` | the db-tier services, the Trunk build and the relay, then `just test-browser` |
| `deny` | `cargo deny`: advisories, licences, sources, and the capability bans |
| `coverage` | `cargo-llvm-cov` over the host tier, non-blocking. An `lcov` baseline as an artifact |

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
| Two browser cases of the canvas and DOM boundary | the browser tier boots the whole application and runs commands from the palette to the board. Two cases are left: one IME composition cycle (`CHROME-007`), which a driver cannot compose, and rAF suspension and resumption behind a modal, which no page reads back |
| `ambre-chrome` DOM interactions | the reactive store, the toast and the shell cell are covered. `Dialog` (modal, Escape, backdrop click, the canvas suspend and resume) and `ContextMenu` (keyboard navigation, typeahead, outside close) need richer driving. `CommandPalette`, `Sidebar` and `Toolbar` are unbuilt, and get a belt when they land |
| The GPU "how to draw" half | section 5.B: the upload and the shaders need a real device. The CPU work beside them is extracted and host-tested |
| `ambre-sync` benchmarks, and further pipeline tiers | they land when the pipeline exits placeholder |

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
