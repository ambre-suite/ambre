# Spike: the unwrap report as a rustc lint

Roadmap story (E9): "The newtype unwrap check as a rustc lint, or kept on syn by decision".
This directory is a spike. It lives on the branch `unwrap-rustc-lint`, which never merges.
The decision, and its record in `ambre-spec-testing.md`, stay with the maintainer.

## The question and the time box

* **Question.** Can a late lint pass on the pinned nightly report the sites that the syn report of
  `ARCH-073` reports, and at which cost?
* **Time box.** One session.

## What the spike is

| File | Content |
| :--- | :--- |
| `src/main.rs` | A `rustc_driver` binary, 463 lines, no dependency. It registers the unwrap pass and the census pass. |
| `run.sh` | Runs the driver as `RUSTC_WORKSPACE_WRAPPER` on the host line and the wasm lib line. |
| `compare.py` | Matches the lint sites with the output of `ambre-primitives unwrap --ambiguous`. |
| `src/census.rs` | A second late lint pass, 383 lines: the other checks of the primitive gate, as sites. |
| `compare_sig.py` | Matches the `sig` sites of the census pass with `ambre-primitives.tsv`. |
| `fixture/` | One site per syn approximation and per census check, with the expected sites. `run.sh fixture` fails on a diff. |
| `ra/` | The newtype census on the crates of rust-analyzer, on stable. |

The pass reads each body with its typeck results. It applies the rule of `ARCH-073` as the syn
report reads it: the same accessors, the same peels, the same exemptions, the same `impl Into<T>`
boundary. It has no ambiguous verdict, since every receiver has a type and every callee a `DefId`.

The spike is outside the workspace. The driver links `rustc_private` crates, so it builds only
on the pinned nightly with the `rustc-dev` component. It builds from outside the repository: the
`build-std` key of `.cargo/config.toml` links a second `std` beside the one of `rustc_driver`.

## Results

Read on 2026-11-08 on commit `968e85c`, with `spikes/unwrap-lint/run.sh all` and
`cargo run -p ambre-primitives -- unwrap --ambiguous`.

| Verdict | Sites |
| :--- | ---: |
| Refused by the syn report | 20 |
| Ambiguous for the syn report | 137 |
| Refused by the lint | 33 |
| Refused by both | 19 |
| Refused by the syn report alone | 1 |
| Refused by the lint, ambiguous for the syn report | 9 |
| Refused by the lint, unseen by the syn report | 5 |
| Ambiguous for the syn report, cleared by the lint | 128 |

The lint reports every site that the syn report refuses and that the rule refuses:

* **The one syn refusal the lint drops** is `ambre-canvas src/render/atlas.rs:662`, `Px::get ->
  .size`. The callee is the swash `ScalerBuilder::size`, a foreign call. The syn report cannot type
  the receiver. Its line in `ambre-unwrap.tsv` excuses a false refusal.
* **The nine sites hidden as ambiguous** are refusals. Examples: `CellKey::as_str` handed to
  `SheetIndex::place` three times in `ambre-trame src/logic/remote.rs`, and `Payload::as_bytes`
  handed to the tuple struct `LoroBytes` twice in `ambre-murmure src/sequence.rs`.
* **The five sites the syn report never sees** come from three approximations. A type alias,
  `MarkRoot::parse(key.as_str())`, three times in `ambre-prose`. A workspace trait method on a
  primitive, `self.0.scaled(zoom.get())` in `ambre-core src/unit.rs:714`. A receiver typed through
  a field, `f(el.rotation.to_degrees())` in `ambre-toile src/logic/interop/svg_export.rs:127`.
* **The 128 cleared sites** are sites the syn report cannot judge. The lint types each one and
  finds no workspace callee. The largest groups are `as_str` (39), `to_string` (29) and
  `to_usize` (27).
* **The wasm lib line adds no site** to the host line.

## Cost

Measured on 2026-11-08 in the cloud container of the session: 4 vCPU, 15 GB, Linux 6.18.

| Item | syn report | rustc lint |
| :--- | :--- | :--- |
| Code of the unwrap check | `unwrap.rs` 3 093 lines, `unwrap_tests.rs` 1 173 lines, beside a shared `scan.rs` of 5 499 lines | `src/main.rs`, 459 lines |
| Dependencies | `syn`, `quote`, `proc-macro2` | none: the `rustc_private` crates |
| Toolchain | any | the pinned nightly, plus `rustc-dev`: 631 MB on disk |
| Build of the checker, cold | 37.7 s, `cargo build -p ambre-primitives` | 0.8 s, `cargo build --release` of the driver |
| Run, both lines, every workspace unit stale | 7.3 s, `ambre-primitives unwrap --ambiguous` | 20.8 s, `run.sh all`, which drops the fingerprints of the workspace units |
| Run, host line, own target directory, cold | -- | 4 min 18 s, dependencies included |
| Run, wasm lib line, cold | -- | 1 min 7 s, dependencies and `build-std` included |
| Reuse of the lint compiles | reads the dep-info of `just lint` | none: clippy holds the wrapper slot |

The lint can share the target directory of `just lint`. The wrapper changes the workspace units
alone, so most dependencies stay fresh. Measured on the same day, with `LINT_TARGET_DIR=target
run.sh host` right after a cold `just lint` of 5 min 26 s:

| Run | Time |
| :--- | ---: |
| First lint run after `just lint` | 1 min 20 s |
| Next lint run, every workspace unit stale | 9.3 s |
| `just primitives`, warm, for comparison | 13 s to 16 s |

The first run rebuilds the dependencies whose feature set differs from the `--all-targets` line.

## Known limits of the spike

* **The incremental cache keys the lint set.** A driver rebuilt with another lint set made rustc
  fail with an internal compiler error on the old cache. `run.sh` sets `CARGO_INCREMENTAL=0`.
* **A late pass whose lints are all allowed does not run**, and the wasm `rustflags` carry
  `-A warnings`. The lint is `Deny` and emits no diagnostic: it writes one file per unit.
* **The driver calls about twenty internal functions of rustc.** These functions have no stability
  promise, so a toolchain bump can break the driver build. Each such bump costs an edit.
* **Dylint was not tried.** It adds `cargo-dylint` and a `clippy_utils` pinned to one nightly.
  The bare driver needs neither.
* **The strict mode is not built.** The judge of `ambre-unwrap.tsv` stays in `ambre-primitives`.
  The lint writes sites, and a key needs the enclosing `fn` path to match the allowlist format.
* **Out of scope.** The primitive gate of `ARCH-033` and the dep-info check of `ARCH-050` stay on
  syn. A lint makes `ARCH-050` hold by construction, but only when both checks move.

## Recommendation for the unwrap check alone

Adopt the lint as the source of the unwrap sites, and keep the judge of `ambre-unwrap.tsv` in
`ambre-primitives`.

* **Exactness.** The syn report misses 14 refusals and holds one false refusal. The lint has no
  ambiguous verdict, so 128 sites stop asking for a human read.
* **Size.** 459 lines replace the 4 266 lines of `unwrap.rs` and its tests. Each new syn
  approximation stops being a story.
* **The nightly is a sunk cost.** The workspace pins a nightly for `build-std` already. The new
  costs are the `rustc-dev` component, an edit of the driver on a bump, and about 10 s per run.
* **The `rustc-dev` component goes to the CI lint job alone**, never to `rust-toolchain.toml`.
  A contributor then downloads nothing more.
* **The 14 new refusals come first.** Each is removed or named in `ambre-unwrap.tsv` before the
  strict mode reads the lint.

The larger gain is a second story: the primitive gate of `ARCH-033` on the same driver. The
dep-info check of `ARCH-050` then holds by construction, and `depinfo.rs` goes with it.

## Every check of the primitive gate

The maintainer widened the question on 2026-11-08: rely on the compiler, delete the syn
re-implementation, accept the pinned nightly, keep every newtype field private. Four tracks were
measured, in this order: the type system, the configurable lints of clippy, the crates of
rust-analyzer, a rustc driver.

### What each track gave

Read on 2026-11-08 on commit `968e85c`, in the same container, with the commands named below.

* **Track 1, the type system.** Both the census pass and the rust-analyzer census find one
  hand-written newtype whose field is not private: `ByteLen` in `ambre-toile src/logic/mod.rs:43`,
  `pub(crate)`. A second hit, `StatusLabelProps`, is a struct the `#[component]` macro writes. No
  code outside a module reads the field of one of its newtypes. Private fields cost one edit
  today. They do not hold `ARCH-073`: the accessor stays, and the rule is about its call.
* **Track 2, clippy.** A temporary `clippy.toml` with `disallowed-methods` on the three secret
  readers and the three sinks reported 88 calls in 20 files, in `cargo clippy --all-targets`.
  Every call outside a test sits in a module the rules admit, so the verdict equals the syn
  verdict. The 20 files would each carry one `#![expect(clippy::disallowed_methods, reason)]`,
  several of them for test modules alone. Clippy has no lint for a primitive in a signature, a
  `Deref` target, an alias or the unwrap condition.
* **Track 3, rust-analyzer.** `ra/` loads the workspace with `ra_ap_load-cargo` 0.0.349 on stable
  1.97 and finds 84 of the 86 newtypes of `ambre-newtypes.tsv`. The two it misses compile for
  wasm32 alone, and one load reads one target. Version 0.0.358 needs rustc 1.98. Version 0.0.349
  builds only once `salsa` is pinned to 0.28.2, since 0.28.5 breaks it inside one semver range.
* **Track 4, the rustc driver.** `src/census.rs` matches 4 178 of the 4 312 lines of
  `ambre-primitives.tsv` at its first attempt, by crate, file, kind and name. The 134 lines left
  come from three causes: an `impl Trait` parameter, an `async fn` return, and the parameters a
  `#[component]` macro writes again. The census pass also finds the 86 newtypes of the census, beside 45
  structs it does not filter yet as the census does. It finds no secret read outside its module, and zero sites for `deref`, `alias` and `bool-lit`, as the
  syn gate does. The fixture holds one site per check, so each check goes red.

### The table

| Check | Rows | Mechanism | Syn lines it deletes | Sites touched | CI cost | Toolchain |
| :--- | :--- | :--- | ---: | ---: | :--- | :--- |
| Primitive in a signature, the allowlist judge | `ARCH-033` to `ARCH-036`, `ARCH-038` | driver: `sig` sites; the judge of the TSV stays, keyed on the sites | `scan.rs` and its tests, 7 930 | 4 312 keys regenerated once, read by a person | shared with the unwrap run | pinned nightly, `rustc-dev` |
| `bool` fields and literals | `ARCH-039`, `ARCH-040` | driver: `bool-lit`, and the `field` sites with their visibility | inside `scan.rs` | 0 | shared | pinned nightly |
| `Deref` to a primitive | `ARCH-037` | driver: `deref` | inside `scan.rs` | 0 | shared | pinned nightly |
| Unwrap | `ARCH-073` | driver: unwrap pass | `unwrap.rs` and its tests, 4 266 | 14 new refusals, 1 false refusal dropped, 128 ambiguous sites cleared | shared | pinned nightly |
| Newtype census | `ARCH-062` | driver: `newtype` | `newtypes.rs` and its tests, 584 | 0 | shared | pinned nightly |
| Private newtype fields | the newtype rule | driver: `newtype` visibility and `field-read`, refused | none: new | 1, `ByteLen` | shared | pinned nightly |
| Family alias | `ARCH-066` | driver: `alias` | `aliases.rs` and its tests, 682 | 0 | shared | pinned nightly |
| Secret readers, zeroize | `ARCH-045`, `ARCH-046`, `ARCH-048`, `ARCH-049` | driver: `secret`, `zeroize`, judged by module | `secrets.rs` and its tests, 913 | 0 | shared | pinned nightly |
| Walk equals rustc | `ARCH-050` | none: holds by construction | `depinfo.rs` and its tests, 3 023 | 0 | none | -- |
| A `.rs` file no compile reads | `ARCH-033`, the seventh check | a file-level script over the dep-info of the lint run, about 50 lines | inside `depinfo.rs` | 0 | under 1 s | python |
| A feature no compile turns on | `ARCH-033`, `features.rs` | kept: `features.rs` over `cargo metadata` | 0 | 0 | as today | any |

The driver run itself, both lint lines, every workspace unit stale: 22.8 s for the two passes,
against 20.8 s for the unwrap pass alone. Sharing the target directory of `just lint` costs
1 min 20 s on the first run and 9.3 s after it. The `rustc-dev` component weighs 631 MB.

| Track | Build, cold | Run | Reads both targets | Holds the unwrap condition |
| :--- | :--- | :--- | :--- | :--- |
| syn, today | 37.7 s | 7.3 s to 16 s | through the dep-info of clippy | approximately: 143 ambiguous sites |
| clippy configuration | none | about 1 min of clippy again, when the configuration changes | yes | no |
| rust-analyzer 0.0.349 | 2 min, release | 37 s with the sysroot | no: one target per load | not tried |
| rustc driver | 0.8 s | 22.8 s, or 9.3 s in the clippy target directory | yes | yes |

### The recommendation

Move every check that reads a type to the rustc driver. Keep two file-level checks, and delete
the syn walk once the driver holds its fixture and the refusals of the workspace.

* **One mechanism.** The driver holds all seven checks that read types, and only it holds the
  unwrap condition. Clippy holds the secret rules alone, and splitting them off adds a second
  mechanism for 913 lines. Rust-analyzer reads one target per load and broke within one semver
  range of `salsa` during the spike.
* **The nightly cost is one edit per deliberate bump.** The driver calls internal functions of
  rustc, which have no stability promise. The pin makes every break a planned edit, never a surprise.
* **Dylint adds nothing here.** The bare driver needs no `cargo-dylint` and no `clippy_utils`.
* **The allowlists stay as files.** A `#[expect]` per line of `ambre-primitives.tsv` puts 4 312
  attributes in the code. The keys move to the form the driver writes, in one reviewed commit.
* **Private fields go into the driver as a refusal.** One field changes today, `ByteLen`.
* **About 17 000 of the 18 897 lines of `ambre-primitives` go.** The judge of the allowlists,
  `features.rs` and a reader of the driver sites stay.

The rows that a person holds today and the driver holds by construction: `ARCH-043`, sites a
macro writes, which the driver sees expanded; `ARCH-044`, a foreign alias, which resolves; and
`ARCH-050`, the walk against rustc.
