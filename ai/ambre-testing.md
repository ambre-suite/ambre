# AMBRE -- TESTABILITY

> Ambre is testable by design. Every new line must be reachable by an automated check, the UI
> included. The doctrine, the verification regimes and the mechanism ladder are normative in
> `specs/1-Foundations/ambre-spec-testing.md`. This file routes you to a mechanism.

A contract row is written before the code, as `unimplemented`. Its `Mechanism` cell reads
`not built`, and its `Gap` cell lands with the roadmap Story that will build it. The day that code
ships, the row names a `Mechanism` that runs in CI: code ships with its mechanism, or not at all.

A gap found after the code shipped turns the row `failing`, bound to a Story. A row reads
`verified`, `failing`, `unimplemented` or `human-review`, and `just contracts` refuses any other.
A mechanism that no CI job reaches verifies nothing. Status values and the full grammar are
normative in `specs/ambre-spec-template.md`, section 1.

## What you just wrote, and what proves it

| Code | Mechanism | With |
| :--- | :--- | :--- |
| A pure function: formula parser, VM, DAG | Properties, on host | `proptest`. A property needs an oracle: two independent paths that must agree. |
| CRDT state (`trame`, `toile`, `prose`) | Reference model | 2-peer convergence through `MockSyncHandle` and `apply_remote`; an incremental cache against a full rebuild. |
| Command and event routing (`ambre-logic`, `ambre-sync`) | Host unit test | Test `ambre_bus::conduit` channels on the Logic side; `LogicSink::channel` on the sync side. |
| Anything at the ZK boundary | Typing first, negative test second | Make plaintext unrepresentable (`Payload<Ciphertext>`, `Confidential<Plaintext>`). What the type cannot hold becomes a test asserting an absence. |
| `ambre-crypto` | Strict | Round-trip, tampering, zeroize, `proptest`. |
| `ambre-server` | Model-based over the real services | `#[sqlx::test]`. Never mock the database: for a service that is persistence, mocking tests the mock. |
| Draw calls | Recording mock | `RecordingSurface`, on host. |
| A parser on hostile input: `.ambre`, PDF, xlsx, docx, frame decoder | Fuzzing | Not set up yet. Until then, a corpus of malformed inputs as unit tests. |
| GPU pipeline | Assisted review and visual check | `./run_server.sh`, plus `trunk serve`: launch the app and look at it. The row's status is then `human-review`, and its `Gap` names the obstacle (`ambre-spec-testing.md` section 3) whose row says who checks it and when. Extract the decision logic out of `cfg(target_arch = "wasm32")` into pure functions first, and unit-test those on host. |
| DOM glue | Extraction, then the browser | Extract the decision logic to host first. What stays DOM is a wasm-tier test or a driven browser, never `human-review`: the `dom-glue` obstacle is lifted. |

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
* **Postgres, Redis and Garage run under Docker, never on the host**
  (`ambre-spec-testing.md` section 4). The images are the ones `docker-compose.yml` pins, by
  digest. A host `redis-server` or `postgres` is never a substitute, whatever its version. Never
  install a backing service through `apt`, `conda` or a tarball. Never stub one to get green: a
  tier that cannot reach its services reports what it could not run.
* **The db tier runs in a cloud session.** Docker is installed there, so "no Docker" is never the
  reason a tier was skipped. Follow the procedure below.
* **Benchmarks defend the 60 FPS and document open budgets.** Two exist, `ambre-crypto/benches` and
  `ambre-logic/benches`, and `just fixtures-bench` runs the second. No CI job runs either. Until the
  performance budget pass lands, no contract row may cite a bench as its `Mechanism`.

## The db tier in a cloud session

Docker is installed, but its daemon is not started. Docker Hub also refuses anonymous pulls from
the session (`429 Too Many Requests`), so route `docker.io` through Google's mirror before you
start the daemon. The pinned digests resolve unchanged through the mirror.

```bash
# 1. The mirror, then the daemon.
mkdir -p /etc/docker
echo '{"registry-mirrors":["https://mirror.gcr.io"]}' > /etc/docker/daemon.json
(dockerd > /tmp/dockerd.log 2>&1 &)
until docker info > /dev/null 2>&1; do sleep 1; done

# 2. The services: postgres, redis, then garage with its layout, key and bucket.
#    Stop a host redis-server or postgres first, so it does not hold 6379 or 5432.
just services-up

# 3. The proxy exports placeholder AWS keys, and Garage refuses them. CI runs without them.
unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY

# 4. The tier itself. `.cargo/config.toml` gives cargo the addresses of the stack (`[env]`).
just test-db
```

If the session may not write `/etc/docker/daemon.json`, give the mirror to the daemon on its command
line instead. Skip the two first lines of step 1, and start the daemon with this line:
`(dockerd --registry-mirror=https://mirror.gcr.io > /tmp/dockerd.log 2>&1 &)`.

`just services-down` tears the services down. If `just` is missing, run the recipe's lines from the
`justfile` as they stand. The daemon does not survive a restart of the container: when the tests
fail with `Cannot connect to the Docker daemon`, run steps 1 and 2 again.

## The wasm tier in a cloud session

Run `just test-wasm`, with no variable set by hand. The recipe drives Chrome even where the
environment exports `GECKODRIVER`, and names the Chrome of `BELT_CHROME` to its `chromedriver`.
`scripts/wasm_driver.py` holds the decision and the reason for each step.
