# AMBRE -- TESTABILITY

> Ambre is testable by design. Every new line must be reachable by an automated check, the UI
> included. The doctrine, the verification regimes and the mechanism ladder are normative in
> `specs/1-Foundations/ambre-spec-testing.md`. This file routes you to a mechanism.

A contract row is written before the code, as `unimplemented`. Its `Mechanism` cell reads
`not built`, and its `Gap` cell lands with the roadmap Story that will build it. The day that code
ships, the row names a `Mechanism` that runs in CI, or it turns `untested` and keeps its Story.
`unverified` is not a status: `just contracts` refuses it.
A mechanism that no CI job reaches verifies nothing. Status values and the full grammar are
normative in `specs/ambre-spec-template.md`, section 1.

## What you just wrote, and what proves it

| Code | Mechanism | With |
| :--- | :--- | :--- |
| A pure function: formula parser, VM, DAG | Properties, on host | `proptest`. A property needs an oracle: two independent paths that must agree. |
| CRDT state (`trame`, `toile`, `prose`) | Reference model | 2-peer convergence through `MockSyncHandle` and `apply_remote`; an incremental cache against a full rebuild. |
| Command and event routing (`ambre-logic`, `ambre-sync`) | Host unit test | Mock `flume` channels; `LogicSink::channel` on the sync side. |
| Anything at the ZK boundary | Typing first, negative test second | Make plaintext unrepresentable (`Ciphertext`, `SecretBytes`). What the type cannot hold becomes a test asserting an absence. |
| `ambre-crypto` | Strict | Round-trip, tampering, zeroize, `proptest`, plus line-by-line human review. |
| `ambre-server` | Model-based over the real services | `#[sqlx::test]`. Never mock the database: for a service that is persistence, mocking tests the mock. |
| Draw calls | Recording mock | `RecordingSurface`, on host. |
| A parser on hostile input: `.ambre`, PDF, xlsx, docx, frame decoder | Fuzzing | Not set up yet. Until then, a corpus of malformed inputs as unit tests. |
| GPU pipeline, DOM glue | Assisted review and visual check | `./run_server.sh`, plus `trunk serve`: launch the app and look at it. The row's status is then `qa-manual`. Extract the decision logic out of `cfg(target_arch = "wasm32")` into pure functions first, and unit-test those on host. |

## Rules

* **Name the test after the behaviour it proves**, as the corpus does:
  `convergence_tests::locked_element_ignores_move`. The `Mechanism` cell then cites the full
  path `crate::module::test_name`, checked against the runner output, never guessed. A cell may
  also name a type-level construction or a lint instead of a test, or state that no mechanism
  exists. See the template, section 1.
* **A bug found freezes into a named regression test**, never into a property.
* **Bug-fix workflow.** Write the test that reproduces the bug first, when feasible. Then fix the
  bug. Then confirm the test passes.
* **Three tiers:** `just test-native` on host, `just test-db` for the server, `just test-wasm` for
  every `cfg(target_arch = "wasm32")` surface.
* **DB and Redis tests are gated by the Cargo feature `db`, never by a bare `#[ignore]`.** Pattern:
  `#[sqlx::test]` plus
  `#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]`.
  `just test-db` runs them, services already up through `docker compose`. `#[sqlx::test]`
  provisions an ephemeral database per test and applies every migration.
* **Benchmarks defend the 60 FPS and document open budgets.** Two exist, `ambre-crypto/benches` and
  `ambre-logic/benches`, and `just fixtures-bench` runs the second. No CI job runs either. Until the
  performance budget pass lands, no contract row may cite a bench as its `Mechanism`.
