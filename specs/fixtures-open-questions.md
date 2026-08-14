# FIXTURES — OPEN QUESTIONS

Clarification pass over `1-Foundations/ambre-spec-fixtures.md`, five rounds, each round's findings
independently verified against the spec corpus and the codebase before being recorded here.

> **Status: resolved, one exception.** Every finding below has been answered in the spec (and, where it
> belonged there, in `ambre-roadmap.md`) except **E4**, the repo-wide `Draft` status question, which is a
> project-level decision rather than a fixtures one. This file is kept as the record of what was wrong and
> why, so the reasoning behind the answers is recoverable and the rejected claims are not re-raised. Each
> table carries the resolution; where a fix took a design call rather than a wording change, the call is
> stated.

This began as a pure relevé: what is missing or self-contradictory, not what the answer should be.

**Method.** 36 claims raised, of which **26 confirmed**, **5 partially confirmed**, **4 rejected** as
false positives, 1 found already done. A further **7** were surfaced by the verification passes rather
than the review passes. Rejected claims are listed at the end so they are not re-raised.

---

## A. Blocking — `FIXTURES P0` cannot be implemented as written

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| A1 | **No crate is named as the definer of `FixtureGen`, `FixtureManifest`, `FixtureShape`.** The spec puts implementors in `ambre-trame`/`ambre-toile` (§2.B) and the machinery in `ambre-logic`, justified by "`ambre-logic` depends on the apps, not the reverse". Rust's orphan rule then forbids the arrangement: `impl FixtureGen for …` inside `ambre-trame` requires that crate to depend on the trait's home. Verified: neither app crate depends on `ambre-logic`. A crate upstream of both (`ambre-core`, `ambre-bus`) would work, but none is named. | spec §2.B; `crates/*/Cargo.toml` | `ambre-core` — upstream of `ambre-bus`, which both apps depend on; already owns `DocType`, `serde`, `hex`. Types there, behaviour in `ambre-logic`. §6 records the rejected `ambre-logic` placement. |
| A2 | **The frozen belt's central assertion has no defined relation.** "The migrated state compares equal to the live artifact its manifest names" is called "the one with teeth", but no equality is defined, and byte equality is impossible by construction (differing op history, peer ids, timestamps — as I1 itself concedes). No `PartialEq` exists on `TrameState`/`ToileState`/`DocState`. `architecture §5`'s `ARCH P8` belt repeats the same undefined verb ("matches"). | spec §4; `architecture:744` | New `canonical_dump(&DocState) -> String`: every user-visible value, deterministic order, as text. Compared instead of state or bytes; a failure prints a readable diff. |
| A3 | **`FixtureManifest` is not deserializable as specified.** `generator: &'static str` cannot be produced by owned deserialization — serde implements `Deserialize<'de>` for `&'de str` only. Needs `String`. The block also carries no `#[derive(Serialize, Deserialize)]`, which `template §0` lists as expected content for a shape declaration. | spec §2.A:114 | `generator: String`, `#[derive(Serialize, Deserialize, PartialEq)]` on all three types. |
| A4 | **Seed provenance is unspecified.** `build(&self, seed)` receives it and the manifest records it, but nothing says who chooses it, nor that it must be stable across regenerations. An unstable seed makes the drift gate fire on every run. | spec §2.A, §2.B | New `FixtureGen::seed()` — a constant the generator owns. `build()` loses its parameter. |
| A5 | **Nothing can write the manifest.** `build` returns `Vec<u8>`; `shape_of` needs a decoded `&DocState`. Assembling a manifest therefore requires re-importing the bytes just generated — a step absent from the data flow. Worse, the layout table credits the `.manifest` to "a live generator", an entity whose interface cannot produce one. | spec §2.A, §2.B, layout table | The **driver** in `ambre-logic` writes it, re-importing the fresh bytes to compute `shape_of` — which doubles as proof the artifact loads before it is committed. Layout table re-credited. |

---

## B. Internal contradictions

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| B1 | **Fixture code ships in the wasm client.** `shape_of` returns a `FixtureShape` — a type only this spec defines — and lives in `ambre-logic`, which `ambre-app` (`crate-type = ["cdylib"]`) depends on unconditionally. This contradicts I4 ("no fixture code ships in a client binary") and §5 ("not in the `ambre-app` dependency graph"). §5's feature promise covers only "Generators", never the loader. No boundary is defined between production code serving fixtures and fixture code proper. | spec I4, §5; `ambre-app/Cargo.toml` | I4 rewritten to draw the boundary **by symbol, not by crate**: everything is fixture code except `DocState::import_snapshot` / `export_snapshot`, which are production. The `fixtures` feature covers `ambre-logic` too. |
| B2 | **"Never a silent golden refresh" is unenforceable.** `just fixtures` rewrites the goldens in place with no gate; committing afterwards makes `fixtures-check` pass trivially. The only integrity checks are self-consistency; nothing compares a new digest against the previously committed one. | spec §4, §2.B | Dropped the claim. The gate compares corpus against code, never against the corpus's history, so it cannot enforce this. Restated as a review rule whose artifact is the JSON manifest diff; §6 records why the machine check was rejected. |
| B3 | **The Definition of Done excludes `FIXTURES P2` from ever completing.** "No phase ships `[x]` without its belt green in CI", yet P2's benches run on git-ignored corpora that "no CI job needs". P2 is neither carved out nor given an alternative criterion. | spec §4 | P2 carved out explicitly, with a stated substitute: `just fixtures-bench` clean end to end plus baselines committed with their machine identity. |
| B4 | **A `generator_version` bump breaks the frozen belt.** The comparison in A2 holds only if the generator's output has not changed; `generator_version` exists precisely to record that it has. After any bump the frozen artifact fails a belt meant to test migration alone. The interaction is never addressed. | spec §2.A, §4 | On a `generator_version` mismatch the semantic comparison is **retired, not repaired** — the first two assertions stand and the artifact reports as degraded. Stated as a real, unavoidable coverage loss. |
| B5 | **The round-trip belt names a method that does not exist.** "`export_snapshot` -> `import_snapshot` -> `export_snapshot` on **every** `DocState` variant", but `DocState` has no `export_snapshot` — only the per-app states do, plus `LogicState::export_snapshot(document_id)`. P0 does not add one. | spec §4:334; `ambre-logic/src/data.rs:310` | `DocState::export_snapshot` added to §2.B and to `FIXTURES P0`. |
| B6 | **`name()` returns "the artifact path"; `generator` is documented as the stem** — "the part before the `@` in `frozen/`". Same string, two definitions. | spec §2.A:110, §2.B:184 | Aligned on **stem** in both places. |
| B7 | **I3 literally excludes frozen artifacts.** "A committed fixture is produced by a generator from a seed", while §4 insists no current generator can produce a frozen one. Defensible in substance, unreconciled in wording. | spec I3, §4 | I3 rewritten: every committed artifact traces to a generator run — directly for `current/`, by copy for `frozen/`. |

---

## C. Undefined behaviour

| # | Finding | Resolution |
|---|---|---|
| C1 | `fixtures-check` is undefined for a generator with no committed golden (newly added) and for a golden whose generator was deleted (orphan). | Both fail the gate, with the reason for each spelled out. |
| C2 | I1's fallback to a semantic digest leaves the gate undefined — `fixtures-check` is specified solely as a byte diff. The spec flags this as deferred rather than resolving it. | P0 now carries the redefinition: adopting the fallback also redefines what `fixtures-check` compares. |
| C3 | `generator_version` has no mechanical effect. It is absent from the manifest-integrity list and nothing states that `fixtures-check` reads it. | Stated as author-facing only — no recipe reads it; it exists for the reviewer reading the manifest diff. |
| C4 | **No corpus lookup API for consumers.** Nothing answers "give me fixture `trame/formula_dag_deep`" — no `load_fixture(name)`, no index, no convention beyond raw paths. This is the most usage-facing omission. | New `load_fixture(name) -> Result<(DocumentId, DocState), FixtureError>`. |
| C5 | `shape` is given three different purposes — test selection (field comment), golden-diff readability (struct comment), manifest integrity (§4) — and no selection mechanism exists anywhere. | Reduced to one purpose (legible golden diff, checked against `shape_of`), with an explicit denial that it is a selection index. |
| C6 | Nothing says how a frozen artifact finds its counterpart in `current/` once the generator is renamed or deleted, which a superseded generator will eventually be. | Degrades exactly like a `generator_version` mismatch, and is dropped at the next retention pass. |
| C7 | `build` is infallible by signature. No policy for `handle_command` failing mid-build, though I2 requires driving it "exactly as the UI does". | `build` stays infallible and **panics** on an unexpected `handle_command` outcome: a half-built corpus is worse than a failed run. |
| C8 | Digest scope is unstated: header-inclusive or payload only. If payload only, the write-once belt cannot detect a tampered `document_id` or `doc_type` byte. | Digest covers the **whole `.ambre` file, header included**, so a flipped `document_id` is caught. |
| C9 | The `fixtures/bench/` row names no filename pattern; whether bench corpora carry manifests is unstated. | `fixtures/bench/<name>.ambre` + `.manifest` — bench corpora carry manifests like everything else. |
| C10 | The load narrative omits `DocState::new(doc_type)` before `import_snapshot`. (The `None` case is out of scope — Prose/Murmure are excluded by §1.A.) | `DocState::new(doc_type)` added to the narrative and the diagram; `None` surfaces as `FixtureError::NoEngine`. |

---

## D. Scheduling, tooling, and reality gaps

| # | Finding | Resolution |
|---|---|---|
| D1 | **`ARCH P8` is an undeclared dependency of the manifest.** `schema_version` is required from P0; the register is built under `ARCH P8`, which the roadmap schedules *after* all three FIXTURES stories. The default is answerable ("absent means 0", `architecture §2.D`) but is not stated here, and §7 gates P0 only on `DOCFILE P0`. | Soft gate stated in §7: generators write 0 until `ARCH P8`, which is what an absent register reads as. The payload migration axis is what actually waits. |
| D2 | **The feature gate is never a deliverable.** §5's entire wasm-safety argument rests on generators sitting behind a non-default feature; no P0 or P1 item creates it or wires it into any `Cargo.toml`. | The `fixtures` feature is now a `FIXTURES P0` deliverable naming all four crates. |
| D3 | **`test-native` cannot run the gate.** The recipe is `cargo test --workspace` with no `--features`. No `fixtures*` recipe exists in the justfile at all. Nothing states how the feature gets enabled for CI. | All recipes pass `--features fixtures`; `test-native` gains an explicit `just fixtures-check` line. |
| D4 | **Benches have no runner.** No bench recipe in the justfile, no bench job in CI. "Fails review" presupposes numbers nobody is specified to produce. | New `just fixtures-bench`, local and on demand; the change author produces the numbers. |
| D5 | **Committed criterion baselines are machine-dependent.** No reference machine, no normalisation, no statement that benches are local-only. Pre-existing repo-wide gap that this spec inherits. | Baselines are committed **with the machine that produced them**; the 20% rule is same-machine only. |
| D6 | **The frozen set grows without bound against a fixed 10 s budget.** §5 caps bytes, not count; `fixtures-check` loads and migrates every frozen artifact. No pruning rule, and no statement of what dropping one costs in coverage. | Retention rule: keep the newest three per `doc_type`. §5 states the 10 s budget holds only because of it. |
| D7 | **Freezing has no tooling.** Every other corpus operation has a `just` recipe; freezing is a manual copy that must encode the right versions in the filename, for an obligation `docfile §2.A` calls normative. | New `just fixtures-freeze <doc_type>`, deriving the `@v…-s…` suffix from the manifest rather than from a human. |
| D8 | **`CONTRIBUTING.md` does not exist.** `README.md:131` marks it "(to be written)". The P1 deliverable and §3's synthetic-only rule both point at it. | Creating `CONTRIBUTING.md` is now itself a `FIXTURES P1` deliverable, not an assumed precondition. |
| D9 | **The P1 freeze-step checkbox is stale** — both target locations already carry the obligation in normative language. | Checked `[x]` with a note that both targets already carry it. |
| D10 | **`serde_json` is new to `ambre-logic`.** True at workspace scope, but the crate hosting the manifest has no such dependency today; the spec gives no signal that its `Cargo.toml` needs an edit. | §1.B states the one new dependency edge explicitly; P0 carries the `Cargo.toml` edit. |
| D11 | **`trame-logic` / `toile-logic` are not obliged to document the new pinned-`PeerID` constructor**, though the fixtures spec calls it a public API change in both engines. | P0's determinism-seam item now carries a matching update to `trame-logic §2` / `toile-logic §2`. |

---

## E. Precision

| # | Finding | Resolution |
|---|---|---|
| E1 | `digest: [u8; 32]` serialises as 32 JSON integers, undercutting the reviewability that justified JSON. A hex adapter is the obvious fix; none is mentioned, and no hex helper exists in `ambre-crypto`. | `#[serde(with = "hex")]` — `hex` is already an `ambre-core` dependency. |
| E2 | §5's "the interop export sources" does not distinguish the small committed P1 set from the large git-ignored P2 export fixtures, inviting the reading that all of them fit the ~5 MB cap. | §5 now says the **small** P1 sources are in the cap and the budget-sized P2 ones are git-ignored. |
| E3 | `ambre-roadmap.md`'s `FIXTURES P2` story omits a deliverable the spec lists: generators for `ambre-prose` / `ambre-murmure`. The roadmap restates every phase, so the two drift by construction. | Roadmap P2 gains the missing bullet; P0 and P1 stories re-synced with §7 in the same pass. |
| E4 | Every spec in the repository is `Status: Draft`; none has been promoted to `Reviewed` or `Frozen` though `template §0` defines the enum. Normative obligations are cited between Draft documents throughout — a project-wide workflow gap, not specific to this spec. | **Not fixed — out of scope.** Every spec in the repo is `Draft`; promoting this one alone would be noise. A status-promotion workflow is a project-level decision. |

---

## Rejected — do not re-raise

* **Toile has no "frame" concept.** False. `Frame` is a first-class `ElementKind` (`toile-logic:204`, `:777`); presentation slides *are* frames.
* **§6 rejects RON/JSON while the manifest is JSON.** No contradiction: the row rejects a textual encoding for the *document body*, which would duplicate the CRDT schema. The manifest is a metadata sidecar.
* **`DocType` may lack serde derives.** It has them (`ambre-core/src/doc_type.rs:13`).
* **The Rust blocks violate `template §0`.** They comply — type and signature declarations only, no function bodies.
