# AMBRE — SPEC: [COMPONENT_NAME]
**Type:** [Horizontal (Layer/Engine) | Vertical (Feature/Widget)]
**Status:** [Draft | Reviewed | Frozen]

> Concise description of the component's single responsibility. Definition of the technical or functional problem being solved.
> **Note:** This template is modular. Ignore or remove irrelevant sections for your scope (e.g., no CRDT for a pure calculation engine).

## 1. OBJECTIVES & CONSTRAINTS
* **Scope:** Strict definition of the component's boundaries. What it handles vs. what it delegates.
* **Dependencies:** List of internal or external crates. Systematic justification for any new dependency.
* **Invariants:** Guaranteed technical properties (e.g., no allocation on the hot path, deterministic computations, memory safety without `unsafe`).

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
* **Test tiers:** Declare which tiers this component touches -- **host** (`cargo test`, x86_64, default), **db** (server integration, feature `db` + docker compose), **wasm** (`wasm-bindgen-test`, headless Chrome, for any `cfg(target_arch = "wasm32")` / Leptos / GPU surface).
* **Unit Tests:** Coverage strategy for pure functions and state machines. State an indicative coverage target (e.g. every predicate branch, every no-op guard, every error path).
* **CRDT convergence:** If the component holds collaborative CRDT state, a 2-peer convergence property-test is **mandatory** (interleaved op sequences via `MockSyncHandle` + `apply_remote` -> identical final states). Otherwise state "N/A (no CRDT state)".
* **Mocks & Environment:** Host seams used (`CommandSink` / `LogicSink` / `Mock*`); wasm via `wasm-bindgen-test`; DB via feature `db` + `docker compose up`. Isolated, headless, no live network.
* **Benchmarks:** `criterion` targets and regression criteria (latency, CPU).
* **Definition of Done (tested):** A feature/MVP is `[x]` **only when** its test belt -- unit + convergence (if CRDT) + at least one relevant integration/wasm test -- is **green in CI**. An untested increment is not done.

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
