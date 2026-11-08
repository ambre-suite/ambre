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
| `src/main.rs` | A `rustc_driver` binary, 459 lines, no dependency. It registers one late lint pass. |
| `run.sh` | Runs the driver as `RUSTC_WORKSPACE_WRAPPER` on the host line and the wasm lib line. |
| `compare.py` | Matches the lint sites with the output of `ambre-primitives unwrap --ambiguous`. |
| `fixture/` | One site per syn approximation, with the expected sites. `run.sh fixture` fails on a diff. |

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

## Recommendation

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
