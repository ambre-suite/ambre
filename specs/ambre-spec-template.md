# AMBRE — SPEC: [COMPONENT_NAME]
**Type:** [Horizontal (Layer/Engine) | Vertical (Feature/Widget)]
**Status:** [Draft | Reviewed | Frozen]
**Implementation:** [implemented | partial | design-only]

> Concise description of the component's single responsibility. Definition of the technical or functional problem being solved.
> **Note:** This template is modular. Ignore or remove irrelevant sections for your scope (e.g., no CRDT for a pure calculation engine).

---

### Header fields (normative)

> `Status` is about **this document** (has it been reviewed). `Implementation` is about **the code it describes**, and a reader must be able to tell within the first five lines whether they are reading a description or a wish. Two specs in the same format, one covering 30 000 shipped lines and one covering a 7-line stub, is the most expensive lie a spec corpus can tell.

* **`implemented`** -- an implementing crate or module exists and this spec describes what it does. Divergences between the two are recorded **in this spec**, not discovered later.
* **`partial`** -- part of the surface is built; the roadmap section marks which increments landed. The default for a spec under active construction.
* **`design-only`** -- no implementing code beyond a stub. This spec is a proposal, and it carries an invariant table like any other, every row `unimplemented` and bound to the roadmap Story that will build it (architecture §4). This is where the table earns the most: a design is far easier to review as twenty numbered, falsifiable rules than as twenty pages of prose, and contradictions inside it become visible instead of staying buried.

---

## 0. CODE IN SPECS (CONVENTION)
> A spec states **what** and **why**; the source states **how**. Duplicating implementation here creates a second, uncompiled copy that silently drifts from the code. Follow this rule in every section below.

* **Allowed (this is the contract):** type and data-model definitions -- `struct`, `enum`, trait signatures, public API signatures, IPC / event / message shapes, `#[derive(...)]`. These define the shape others must respect and belong in the spec.
* **Allowed (declarative):** invariants, pre/post-conditions, and algorithms described as numbered steps or pseudo-code. Describe the algorithm, not its Rust transcription.
* **Forbidden:** complete function bodies (`fn foo(&mut self, ...) { ... }` with real logic). Replace them with (a) a one-line description of the algorithm, (b) the invariant it upholds, and (c) a pointer to the real code (`// ambre-crate/src/module.rs`).
* **Illustrative only:** a short example test or snippet is fine to clarify intent; the authoritative test suite lives in the repo, never mirrored here.
* **Decision test:** if a block can be copy-pasted into a `.rs` file and compiled as behavior, it is implementation and does not belong in a spec. If it is a shape declaration or a described algorithm, it does.
* **Fences.** Rust -- and only Rust -- goes in a ` ```rust ` fence: shapes and signatures, never a body. Everything else declares its language so the checker can pair fences and never mistake code for prose: diagrams, trees, wire layouts and pseudo-code use ` ``` ` (untagged) or ` ```text `; a genuine non-Rust artefact uses its own tag (` ```sql `, ` ```toml `, ` ```yaml `, ` ```css `, ` ```json `, ` ```html `, ` ```bash `). An embedded runnable script (e.g. `lua`) is implementation, not a shape -- state its algorithm as pseudo-code in a ` ```text ` fence and point to the source. Every fence must be closed; a Rust shape left in an untagged fence is a tagging bug, not an exemption.
* **Mechanised.** `just spec-code` (CI job `lint`) fails on any function body inside a ` ```rust ` fence in `specs/`, on any Rust shape sitting in a non-`rust` fence, on a disallowed fence tag (`lua`, ...), and on an unterminated fence. Keep the signature and its doc comment, collapse the body to `{ /* ... */ }`, and put the algorithm in the prose around it -- numbered steps or a table when it is a mapping. The rule was written long before it was enforced, and the corpus had accumulated 1 466 lines of bodies in the meantime.

## 1. OBJECTIVES & CONSTRAINTS
* **Scope:** Strict definition of the component's boundaries. What it handles vs. what it delegates.
* **Dependencies:** List of internal or external crates. Systematic justification for any new dependency.
* **Invariants:** Numbered, falsifiable properties this component guarantees (e.g., no allocation on the hot path, deterministic computation, no plaintext crossing a transport boundary). Fill the table below. The grammar, the falsifiability test and the status values are normative in `ambre-spec-architecture.md §4` ("Invariants (normative)") -- reference it, do not redefine it.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `XXX-001` | The component MUST ... | this spec §2.B \| `derived` | `crate::module::test_name` | verified |
| `XXX-002` | The component MUST NOT ... | README pillar "E2EE" | -- | unverified |
| `XXX-003` | The component MUST ... | this spec §4.A | not built -- lands with `E5` / `ARCH P2` | unimplemented |

> `XXX` is this spec's stable prefix, declared once and never renumbered -- it is the join key between spec, test name and audit finding. Prose that cannot be falsified ("the engine is robust") is an intention, not an invariant: it belongs in **Scope** above. Every row carries a `Status`; `unverified` and `unimplemented` are legitimate values, an empty cell is not. A rule for code that does not exist yet is written as a row (`unimplemented`), never held back and never left as prose -- `ambre-spec-architecture.md §4` carries the two conditions that keep it honest.

---

## 2. TECHNICAL DESIGN & STATE

### A. Data Modeling
* **CRDT Structure (Loro):** If the component is collaborative, definition of primitive layout (Map, List, Text, MovableTree).
* **Persistence Schema:** Definition of `Postcard` structures for local or remote storage.

### B. Interface Contract (API)
* **Traits & Public API:** Definition of public methods, implemented traits, and input/output types.
* **Data Flow:** Diagram of message passing (Flume Channels) between the Logic Worker and this component.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)
* **Boundary Audit:** Identification of outbound flows. Proof that no plaintext data reaches the Blind Relay or storage.
* **Cryptography:** Detail of RustCrypto primitives used (XChaCha20-Poly1305, Ed25519) and local key management.

---

## 4. QUALITY & TESTABILITY
> Prescriptive. Every spec MUST fill in each item below. See the transverse test architecture in `ambre-spec-architecture.md §4` (test tiering, CRDT convergence, CI, coverage) -- reference it, do not redefine it.
* **Invariant coverage:** every invariant declared in §1 carries a `Verification` entry -- a concrete mechanism (test path, type-level construction, lint) or the explicit absence of one (`human-review`, `qa-manual`, `unverified`). The ID is the join key in both directions: the test name cites it, the invariant names the test. The cross-spec status roll-up lives in `ambre-spec-testing.md §7`.
* **Test tiers:** Declare which tiers this component touches -- **host** (`cargo test`, x86_64, default), **db** (server integration, feature `db` + docker compose), **wasm** (`wasm-bindgen-test`, headless Chrome, for any `cfg(target_arch = "wasm32")` / Leptos / GPU surface).
* **Unit Tests:** Coverage strategy for pure functions and state machines. State an indicative coverage target (e.g. every predicate branch, every no-op guard, every error path).
* **CRDT convergence:** If the component holds collaborative CRDT state, a 2-peer convergence property-test is **mandatory** (interleaved op sequences via `MockSyncHandle` + `apply_remote` -> identical final states). Otherwise state "N/A (no CRDT state)".
* **Mocks & Environment:** Host seams used (`CommandSink` / `LogicSink` / `Mock*`); wasm via `wasm-bindgen-test`; DB via feature `db` + `docker compose up`. Isolated, headless, no live network.
* **Benchmarks:** `criterion` targets and regression criteria (latency, CPU).
* **Definition of Done (tested):** A feature/MVP is `[x]` **only when** its test belt -- unit + convergence (if CRDT) + at least one relevant integration/wasm test -- is **green in CI**, and **no invariant in §1 is left without a `Status`**. An untested increment is not done; an invariant with an empty status is not a spec.

---

## 5. PERFORMANCE & RESOURCES
* **Performance Budget:** Maximum execution time per typical operation (e.g., <16ms for wgpu rendering at 60 FPS, <1ms for vertex batch build in steady state).
* **WASM Impact:** Evaluation of binary size and heap memory footprint.

---

## 6. REJECTED ARCHITECTURAL DECISIONS
* List of rejected technologies or approaches.
* Precise technical justification for rejection (misalignment with AMBRE pillars, metadata leakage, excessive complexity).

---

## 7. INTEGRATION PLAN (ROADMAP)
* **Phase 0 (Core):** Pure business logic and fundamental tests. Per the tested Definition of Done (§4), no phase ships `[x]` without its test belt green in CI.
* **Phase 1 (Integration):** Hooking into the Logic Worker, DrawSurface (WgpuSurface / KrillaSurface), or the Blind Relay.
* **Phase 2 (Optimization):** Performance refinement and edge case handling.
