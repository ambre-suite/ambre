# AMBRE — SPEC: Test strategy realization & coverage debt
**Type:** Horizontal (Foundation — Quality)
**Status:** Draft
**Implementation:** implemented

> Operational companion to `ambre-spec-architecture.md` §4 ("Quality & Testability"). §4 defines the **normative** three-tier model (`host` / `db` / `wasm`), the per-crate strategy, and the CI shape. This spec records the **realization**: what is actually wired per crate today, the seams used to keep logic host-testable, and the **deliberate, deferred coverage debt** so it is tracked rather than rediscovered. When this spec and §4 disagree on intent, §4 wins; this spec only reports state.

---

## 1. TIERS (recap, normative source = architecture §4)

* **host** — `cargo test --workspace` on x86_64 (`just test-native`). Default tier, zero external dependency. Pure logic, the reference tier.
* **db** — `ambre-server` integration behind Cargo feature `db` (`just test-db`); postgres/redis/minio via compose, `#[sqlx::test]` per-test database.
* **wasm** — `wasm-bindgen-test` for every `cfg(target_arch = "wasm32")` surface (DOM/Leptos, GPU), run under headless Chrome via `wasm-pack test --headless --chrome <crate>` (`just test-wasm`). Slow (`-Z build-std` rebuilds `std`, Chrome startup), so the **host-extraction** seam (§4) pushes the `cfg(wasm32)` boundary as close to the DOM/GPU as possible and unit-tests the extracted pure functions on host.

`just test-wasm` runs the wasm tier for `ambre-trame`, `ambre-chrome`, and `ambre-app`.

---

## 2. REALIZATION PER CRATE

| Crate | host | wasm | Notes |
|---|---|---|---|
| `ambre-core` | ✓ (28) | — | Types. |
| `ambre-crypto` | ✓ (12, proptest) | — | Round-trip, tampering, zeroize. |
| `ambre-protocol` | ✓ (6) | — | Postcard wire round-trip. |
| `ambre-bus` | ✓ (`tests/host.rs`) | — | `to_tsv`, `has_decimals`, `RequestId` monotonic/unique, **Logic<->Sync envelope postcard round-trips** (`SyncCommand`/`LogicEvent`, `id` echo preserved). |
| `ambre-logic` | ✓ (10, flume mock) | — | Typed dispatch. |
| `ambre-sync` | ✓ (23) | — | Crypto pipeline, `MockBlindRelay`/`MockWsTransport`. |
| `ambre-canvas` | ✓ (7) | (GPU not unit-tested, by design — §4) | TextCache, dirty/RepaintPlan, request registry, fatal. |
| `ambre-commands` | ✓ (7) | — | Registry, keybindings. |
| `ambre-trame` | ✓ (269) | ✓ (`tests/chrome_ui.rs`) | Formulas, convergence (proptest), render via `RecordingSurface`; chrome reactive + DOM belt. |
| `ambre-chrome` | ✓ (`key_from_dom`, `place_menu`) | ✓ (`tests/chrome_ui.rs`) | See §3. |
| `ambre-app` | — | ✓ (`route`/`base64`/`tick_fps` inline) | See §4. |
| `ambre-server` | ✓ (91) | — (db tier) | Routes, ACL, Lua Redis, JWT. |
| `ambre-prose`/`-toile`/`-murmure` | — | — | Stubs (no logic yet). |

---

## 3. `ambre-chrome` — extraction seam applied

The shared chrome toolkit had **zero** tests; it now has both belts.

**Host tier (pure decision logic, extracted per §4):**
* `command::key_from_dom(&str) -> Option<Key>` — the DOM-key classification. `keybinding_from_event` is reduced to wrapping it with the modifier bits off a `web_sys::KeyboardEvent` (the irreducible DOM half). Tests: named keys, `F1..F24` bounds, single printable char, dead-key/IME/multi-char rejection.
* `context_menu::place_menu(x, y, w, h, vw, vh) -> (f64, f64)` — the viewport-flip geometry, extracted out of the placement `Effect` (which keeps only the `getBoundingClientRect` / `inner_width` DOM read). Tests: no-overflow, right/bottom/corner flip, menu-larger-than-viewport clamp-to-origin.

`ambre-chrome` compiles on host (leptos CSR + `web-sys` types), so these run under `just test-native`.

**wasm tier (`tests/chrome_ui.rs`, headless Chrome):**
* Reactive belt — `CommandStates` over real leptos signals: defaults (`toggled=false`, `enabled=true`), single-writer propagation, unknown-id no-op.
* DOM belt — `ToastHost` ARIA live-region attributes + kind class/text; `AppShell` renders the named `<canvas>` cell + grid regions.

---

## 4. `ambre-app` — wasm-only crate, pure helpers covered in wasm

The whole crate is `#![cfg(target_arch = "wasm32")]` (web-sys/wgpu/leptos), so it is empty on host and its pure helpers can only run on the wasm target. They are covered as inline `#[wasm_bindgen_test]` unit tests (run under node-less headless Chrome; `wasm_bindgen_test_configure!(run_in_browser)` declared once in `lib.rs` under `#[cfg(test)]`):
* `route::parse` / `doc_path` — path<->`Route` mapping: each doc-type slug, round-trip, unknown slug / malformed id / extra segments fall back to `Landing`.
* `host::base64_encode`/`base64_decode` — clipboard transport codec round-trip + malformed rejection.
* `FrameHost::tick_fps` — FPS sliding window: needs two samples, computes the rate, evicts samples older than 1 s.

> `#[wasm_bindgen(start)] fn main` is gated `#[cfg(not(test))]` so the test harness's generated entry point does not collide ("entry symbol `main` declared multiple times").

---

## 5. COVERAGE DEBT (deferred, deliberate)

Tracked here so it is a decision, not an omission. None blocks the Trame MVP.

1. **`ambre-app` end-to-end headless boot.** §4 lists `ambre-app` integration as "End-to-end headless". Not implemented: a real boot spawns the Logic `wasm_thread` + the separate Sync worker (a second wasm artifact loaded via `ambre-sync_loader.js`) and a wgpu surface — none of which the `wasm-bindgen-test` harness provisions (no COOP/COEP shared-memory page, no worker loader, no GPU adapter). Current `ambre-app` wasm coverage is the **pure helpers** above. A true e2e belt needs a browser-automation harness (Playwright against a Trunk-served build) outside the unit-test runner — reserved for when the boot path churns.

2. **`ambre-chrome` DOM interactions not yet asserted.** Covered: reactive store, toast ARIA/text, shell canvas cell. **Not covered (DOM behaviour, would need richer `wasm-bindgen-test` driving):**
   * `Dialog` — `show_modal()` / Escape / backdrop-click round-tripping the `open` signal, `on_open`/`on_close` firing (canvas rAF suspend/resume).
   * `ContextMenu` — keyboard nav (Arrow/Home/End/Enter/Escape), first-letter typeahead, outside-pointerdown close. (The placement geometry is host-covered via `place_menu`.)
   * `CommandPalette` / `Sidebar` / `Toolbar` — not yet built (Phase 1/2 of `ambre-spec-chrome.md §7`); test when they land.

3. **GPU "how to draw" (`ambre-canvas` render pipeline).** Unchanged from §4: `QuadRenderer`/`WgpuSurface`/`GlyphAtlas` need a real `wgpu::Device`; validated visually (`/run`), not unit-tested. Reserved options (headless software adapter, golden-image readback) only for a precise critical need.

4. **`ambre-sync` benchmarks + further pipeline tiers** land when the pipeline exits placeholder (already noted in architecture §4).

---

## 6. CI

`.github/workflows/ci.yml` jobs `lint` / `test-native` / `test-db` / `test-wasm` / `coverage`, each calling the matching `just` recipe for local/CI parity (architecture §4, §7 ARCH P5). `test-wasm` now exercises `ambre-trame`, `ambre-chrome`, and `ambre-app`.

---

## 7. INVARIANT STATUS LEDGER

> Roll-up of the per-spec invariant tables (grammar: architecture §4, "Invariants (normative)"). The invariants themselves live in the spec of the component they constrain; this section reports **state**, never intent.

> **Derived, not hand-counted.** `just invariants` (`scripts/invariants.py`, CI job `lint`) reads every table and prints the numbers below. It also enforces what keeps them meaningful: a status must be one of the five declared values, an `unimplemented` row must name the Story that will build it, and an id must never be reused or renumbered. The counts in this section were hand-maintained for exactly one week before being wrong by one -- the same failure mode as a hand-maintained test list, and the same fix.

**288 invariant rows, split by whether the code exists.** The split is the whole point: one number is a debt, the other is a plan.

| | Rows | Meaning |
|---|---|---|
| **Shipped surface** | **163** | code exists. `verified` 140, `unverified` 21, `human-review` 2, `qa-manual` 0. **The 21 are the debt** -- shipped code nothing would catch. |
| **Specified, not built** | **125** | `unimplemented`. Rules written before the code, each bound to the Story that will build it. Not a risk; work. |

**Three passes over shipped code:**
* **The ZK boundary (35).** `ZK-001..008` (architecture §3, the boundary itself), `SYNC-001..013` (`ambre-spec-sync.md §1`), `SRV-001..014` (`ambre-spec-server.md §1`).
* **The Trame vertical (61).** `TRAME-001..024` (`trame-logic §1`, the engine), `TRAME-UI-001..015` (`trame-ui §1`), `TRAME-FEAT-001..013` (`trame-features §0`), `TRAME-IO-001..009` (`trame-interop §1`). The second pass of the seeding order below, taken on the largest body of shipped code in the workspace (about 30 000 lines) and the best-tested one, so it measures the grammar against material that already passes.
* **The Toile vertical (67).** `TOILE-001..023` (`toile-logic §1`, the engine), `TOILE-UI-001..021` (`toile-ui §1`), `TOILE-FEAT-001..010` (`toile-features §0`), `TOILE-IO-001..013` (`toile-interop §1`). The third pass, on the second app crate (about 23 000 lines).

**A fourth pass over unbuilt surface (125).** The `unimplemented` rows: `trame-features` 11 (personal filter views, conditional formatting, validation, protection), `trame-interop` 11 (xlsx, PDF), `trame-ui` 8 (multi-user cursors, sparklines, charts, PDF), `toile-features` 7 and `toile-logic`/`toile-ui` 3 each (the comments CRDT, its anchoring, its panel), `toile-interop` 6 (Figma/Miro, PNG render-back, Web-Showcase), and **76 across the four Prose specs**, whose crate is a `lib.rs` and nothing else.

| Status | ZK | Trame | Toile | Prose | Total | Reading |
|---|---|---|---|---|---|---|
| `verified` | 24 | 53 | 63 | 0 | 140 | a named test, a `proptest`, a type-level construction, the compiler, or a `deny.toml` capability ban |
| `human-review` | 2 | 0 | 0 | 0 | 2 | `ZK-006` / `SRV-001` (no ciphertext in logs) -- reviewer attention, by decision |
| `unverified` | 9 | 8 | 4 | 0 | 21 | code exists, nothing would go red; each is listed below |
| `qa-manual` | 0 | 0 | 0 | 0 | 0 | -- |
| `unimplemented` | 0 | 30 | 19 | 76 | 125 | **counted apart** -- see the split above |

Per spec, on the shipped side: `trame-logic` 19/24 `verified`, `trame-ui` 12/15, `trame-features` 13/13, `trame-interop` 9/9; `toile-logic` 21/23, `toile-ui` 19/21, `toile-features` 10/10, `toile-interop` 13/13.

**Executed on the ZK pass:** `ambre-crypto` 19/19 and `ambre-sync` 57/57 green on the host tier; `ambre-server` 94 green with 62 ignored -- the `SRV-*` entries citing `routes::tests::*` are db-tier tests, run by the CI `test-db` job rather than by `cargo test`. Every name in a `Verification` cell resolves to a `fn` in the tree. That check turned out to be too weak -- it does not say the `fn` is reachable, and two `SRV-*` citations named the wrong module for tests behind a Cargo feature no job enabled; see "Feature-gated tests that no CI job could reach" below.

Five of the 24 are held by the **crate graph** rather than by a test (`ZK-002`, `ZK-003`, `ZK-004`, `SRV-012`, `SYNC-011`), and `deny.toml` now refuses the edit that would break them: each capability crate (AEAD, Argon2, X25519, Ed25519, `ambre-crypto`, `loro`) carries a `wrappers` list of the crates allowed to depend on it, checked by the CI `deny` job. Re-granting a capability is an edit of `deny.toml`, reviewed as such, rather than one line in a `Cargo.toml`.

**Executed on the Toile pass:** `cargo test -p ambre-toile --lib` -- **341 passed, 0 failed**; with `--features pdf`, **349 passed, 0 failed**. The 67 rows cite **240 distinct test paths**, every one green in that second run. Four `verified` rows are held by something other than a named test: `TOILE-001` (private `doc`, no public signature naming `LoroDoc`), `TOILE-002` (outcome by value, no async path), `TOILE-UI-001` (no `loro` reference under `src/ui/` outside comments) and `TOILE-IO-013` (crate graph plus the `deny.toml` capability wall). `TOILE-018` is held by the CRDT schema itself -- one `LoroValue::Binary` per element under the `"style"` key -- with concurrent restyles driven by the convergence proptest.

**Executed on the Trame pass:** `cargo test -p ambre-trame --lib` -- **346 passed, 0 failed**. The 61 rows cite **112 distinct test paths**, every one of which appears in that run's green list -- matched mechanically against the runner output, not by reading.

Eleven of the 52 `verified` rows are held by something other than a named test, which is the ladder of architecture §4 working downward from the top: three by a `proptest` (`TRAME-013`, `TRAME-014`, `TRAME-015`), five by construction (`TRAME-001` private `doc` with no public signature naming `LoroDoc`; `TRAME-002` outcome returned by value, no async path; `TRAME-UI-002` a single outbound sender; `TRAME-UI-003` `try_send` only; `TRAME-FEAT-001` no sink on `TrameState`), one by the compiler (`TRAME-UI-006`, an exhaustive match with no `_` arm) and two by the crate graph (`TRAME-UI-001`, `TRAME-IO-001`). None of those eleven can regress without failing a build.

### Divergence report -- ZK boundary

The point of the exercise, and the part that does not come out of a test run.

1. **`ZK-004` / `SRV-012` -- the code exceeded the spec. Closed.** `ambre-server/Cargo.toml` declared `chacha20poly1305`, `x25519-dalek` **and** `sharks` as runtime dependencies; none appeared anywhere in its `src/`. The spec's static check was phrased as "imports no `ambre-crypto`", which the code satisfied -- while the intent behind it (no decryption, key-exchange or secret-sharing capability inside the relay process) did not hold. The falsifiable restatement earned its keep on the first pass. The three declarations are gone and `deny.toml` refuses their return; both invariants are now `verified`. Note for the day Shamir quorum recovery lands (`routes/orgs.rs`, N1): counting `k` encrypted shares before releasing a blob needs no `sharks` dependency, and reconstructing a secret server-side would be a zero-knowledge decision requiring a spec change first.
2. **`SYNC-001` -- the flagship invariant was claimed static and is held by nothing.** `ambre-spec-sync.md` asserted a "statically verifiable property" over a `PlaintextPayload` type that **does not exist** in the tree, and `Ciphertext(pub Vec<u8>)` accepts any bytes from anywhere. The strongest fix is type-level, per the mechanism ladder: give the sealed payload a private field and a constructor reachable only from the crypto pipeline, which makes `ZK-001` and `SYNC-001` compiler-held instead of aspirational.
3. **`SYNC-012` -- an undocumented decision (audit `H3` residue).** `ambre-sync/src/updates.rs` maps `Ok(None) | Err(_)` to `DocumentKey::generate()`, so a transient network failure and a genuinely new document are indistinguishable: the document opens local-only under a key no peer holds. Either the spec adopts this explicitly and states the divergence consequence, or the code separates the two cases. Today it is neither.
4. **`ZK-008` / `SRV-002` -- configuration can violate a spec rule.** The presigned-URL TTL is `state.config.presigned_url_ttl_secs` (default 900 s) with no upper bound enforced, so a deployment can exceed the 15-minute ceiling without any code change. And while role refusals are tested, **no test asserts that a non-member is refused a presigned URL** -- the ACL-before-S3 ordering is verified for roles, not for membership.
5. **`ZK-007` -- no mechanism exists for a whitelist.** "The relay learns nothing beyond this set" has no natural test. Candidate: an `insta` snapshot over the `WsClientMsg` / `WsServerMsg` shapes, so any new plaintext field shows up as a snapshot diff rather than as nothing at all.
6. **The audit ledger lags the code.** `C1` (ciphertext in S3 logs), `C4` (cross-document snapshot access) and `H2` (metadata to non-members) are fixed in the tree but still listed **OPEN** in `security-audit.md`, alongside the `H5` case already noted below. Each now has an invariant citing its ID in `Sources`, which is what stops the two from drifting again. Updating those statuses is a separate pass.

### Divergence report -- Trame vertical

Twelve findings on a body of code that was already the best-tested in the workspace, which is the argument for doing this on green material rather than on suspect material. The first seven are the engine (`trame-logic`), the last five the three sibling specs.

1. **`TRAME-008` -- the spec claims a behaviour the code prevents.** `trame-logic §5` states that a row or column delete propagates `#REF!` to the formulas that referenced the deleted line. The delete path takes the structural reseed, which re-binds every formula **from its raw text**, so the reference lands positionally on the shifted grid and reads an *empty* cell. `logic::tests::group2::formula_reference_is_positional_after_delete` asserts exactly that, and is green. The `BrokenRef` primitive is real and tested at the VM level (`TRAME-007`) but **unreachable from this path**: nothing survives the reseed still holding the deleted `Uid`. `§12` ("References After Delete + Reinsert") describes the same unreachable outcome. The spec has not decided between Excel-like positional behaviour and uid-broken references; it claims one and ships the other. Noted in §5, not silently rewritten -- this is a product decision, not a typo.
2. **`§11 bis.E` was carrying a fixed problem as open debt.** It described masking excluded only for the active sheet and asked for a cross-sheet convergence test. `masked_cells_for(sheet_id, index)` derives the set per sheet and `reseed_engine` excludes it uniformly, and the test it asks for exists and is green (`group3::masked_cell_excluded_across_sheets_regardless_of_active_tab`). Same drift as `§11 bis.F`, in the other direction: an appendix outliving its cause. Marked LANDED, recorded as `TRAME-020`/`TRAME-021`.
3. **`§10 QUALITY & TESTABILITY` was fiction, and it is the general case.** It listed 17 test names; **zero** existed in the tree, while `ambre-trame` carried 346 real tests under entirely different names. Measured across the corpus **before this pass: 82 test names cited in specs, 23 of which resolve to a `fn`** -- `trame-logic` 0/17, `trame-ui` 0/8, `trame-interop` 2/12, `trame-features` 18/30, `ambre-spec-sync.md` 3/11, `prose-*` 0/4. **After it: 4 cited, 0 real**, all four in the Prose specs, where an intended-test list is legitimate -- those specs are `design-only` and describe no code at all. `ambre-spec-sync.md §4` went the same way as the others: its three blocks carried full `#[test]` and `proptest!` **bodies**, an uncompiled second copy of a suite that `ambre-spec-template.md §0` forbids for exactly the reason on display here. A test list in a spec has no join to the suite and no way to fail, so it decays silently; the `Verification` column has both, which is the entire reason for the column. The section is now the tiers, the seams, the mandatory belts and the DoD -- the parts that are not a claim about the tree's contents.
4. **`TRAME-023` -- the engine's core cache invariant is unverified.** Nothing asserts that the incrementally maintained `SheetIndex` equals a from-scratch rebuild after an arbitrary sequence of structural operations; there are per-operation unit tests only. The equivalence property exists on the **Toile** spatial index and not here, on the index every formula resolution goes through. The deleted §10 claimed a `sheetindex_incremental_eq_rebuild` proptest -- the fiction was hiding the gap.
5. **`TRAME-010` -- the invariant a remote peer depends on has no test.** "Computed values never enter the CRDT" is asserted nowhere; only the recompute-after-reseed side is covered. A regression here is invisible locally and corrupts every other peer.
6. **`TRAME-022` -- "a workbook always keeps >= 1 sheet" holds locally only.** The local guard is tested; two peers concurrently deleting the last two sheets still converge to a sheet-less workbook. `§2.D`'s deterministic re-insertion rule is **decided but not implemented**, and the spec already said so -- the invariant table is what makes it countable instead of a parenthesis on line 1832.
7. **`TRAME-024` -- `#![forbid(unsafe_code)]` is still absent** from `ambre-trame`, as from 15 of 17 crates (§8). Measured zero `unsafe`, so the pillar holds today by observation rather than by mechanism.

8. **`trame-ui §14` claimed the opposite of the truth about its own tier.** It opened with "UI tests are run with `wasm-bindgen-test` in a headless browser" and listed eight such tests, **none of which existed**. The real split is 58 **host** tests (`ui::grid`, `ui::renderer`, `render`) against 13 wasm ones. The host-extraction seam of architecture §4 had been applied thoroughly and the spec never noticed -- so a reader planning a new interaction would have reached for the slow tier for work that belongs on the fast one.
9. **`trame-ui §2.B` contradicted itself inside four lines.** The prose says the canvas dispatch carries "no `_ => {}` wildcard, so a new variant is a compile error"; the code excerpt directly under it ended on `_ => {}`. The **code** is right (`ui/view/frame.rs` has no wildcard arm), which makes this the dangerous shape of error: a stale snippet that a reader copies, silently giving up the compile-time check the sentence above it promises. Corrected, and pinned as `TRAME-UI-006`.
10. **`TRAME-UI-005` -- an implemented rule with no test, and a regression that would be misread.** The anti-flicker rule (drop a `ViewportData` older than the last accepted) is in `ui/grid/state.rs` and nothing exercises it. A regression shows up as flicker under fast scroll, which reads as a GPU or rendering problem -- days spent in the wrong crate. The deleted §14 named an `outdated_viewport_version_ignored` test for exactly this; naming it was as far as it ever got.
11. **A test list cannot distinguish a test from a wish, and both specs prove it.** `trame-interop §8` listed 12 names of which **2** existed -- the other ten describe .xlsx and PDF code that has never been written, in the same format as the two real ones. `trame-features §9` listed 30 of which 18 existed, the other 12 belonging to handlers that are explicit no-ops. Nothing in either document marked the difference. Both sections now keep their intent explicitly labelled as intent, and the real coverage lives in `Verification` cells.
12. **Unwritten features carry the same authority as shipped ones.** `trame-ui §9-§12` (multi-user cursors, sparklines, charts, PDF export), `trame-features §3/§5/§6` and `trame-interop §2/§3` are full technical designs for code the crate does not contain -- `ambre-trame` declares neither `calamine`, `rust_xlsxwriter` nor `krilla`. The `Implementation` header field (`ambre-spec-template.md`) fixes this at the document level; **inside** a document, the absence of invariant rows for a section is now the signal, and each of these specs says so where its table stops.

### Divergence report -- Toile vertical

Six findings, and the shape is different from Trame's: the prose enumerations in Toile's quality sections were substantially **true** -- the coverage they demanded exists. What drifted here is not the account of the tests, it is the account of the features.

1. **Eight tests that had never run.** `ambre-toile` keeps `krilla` optional, so `logic::interop::pdf_export` and `logic::state::presentation::pdf_tests` compile only under `--features pdf`. Every CI path (`just test-native` -> `cargo test --workspace`) enables no optional feature, so those eight were invisible: 341 tests ran, 349 exist. They pass. The `justfile` comment above the recipe even asserted that "every gated test lives in `ambre-core` and `ambre-logic`" -- a false statement inside the file whose whole job is local/CI parity. Fixed: `just test-native` now runs `cargo test -p ambre-toile --features pdf`. A test that never executes verifies nothing, so this belonged in the justfile, not in a caveat.
2. **Comments are specified in three places and implemented in none.** `toile-logic §2.J` (anchoring model), `toile-ui §5sexies` (the DOM panel), `toile-features §4.D` (the full thread CRDT schema) -- and `toile-features §6` listed a **mandatory** convergence belt for them. `CreateComment` appears nowhere in the workspace. A belt cannot be mandatory for a feature that does not exist; the roadmap has always said so (`§9`, Phase 2, `[ ]`), while three technical sections read as description.
3. **`toile-interop §1` declares two dependencies the crate does not have.** `serde_json` (Figma/Miro parse) and `lyon` (PNG render-back) are in the table; `ambre-toile/Cargo.toml` has neither, because both features are unbuilt. A dependency table is the most mechanically checkable paragraph in a spec -- `cargo tree` settles it in a second -- which is exactly why it is worth checking: it had quietly become a plan.
4. **`TOILE-015` -- undo local-only is asserted nowhere.** `§8 bis` says Toile "reuses the same model as Trame", and Trame's belt (`undo_local_only_not_remote`) has no counterpart here. The model was reused; the test that proves it was not. This is the cheapest gap in the whole ledger to close.
5. **The two apps have opposite gaps, and neither spec could have told you which.** `TOILE-004` -- the incrementally maintained spatial index equals a from-scratch rebuild -- is held by a real `proptest` inside the 2-peer convergence run. `TRAME-023` is the same invariant on `SheetIndex`, with no test at all. Same architecture, same doctrine, one built the belt and one did not, and before the tables both specs claimed it in prose.
6. **What the exercise did *not* find is worth recording too.** Toile's 341 tests cover every rule its §1 stated in prose, and the four `unverified` rows are two render-loop behaviours nobody exercised (`TOILE-UI-020`, `TOILE-UI-021`), the undo gap above, and the missing `#![forbid(unsafe_code)]`. A spec can be accurate; the point of the grammar is that you no longer have to take that on faith.

### Feature-gated tests that no CI job could reach

Found while seeding Toile, then checked across the workspace, because the failure mode is invisible by construction: a test behind a Cargo feature that nothing enables does not fail, does not get reported as ignored, and does not appear in any count. It simply is not there.

| Crate | Module | Gate | Tests | Ran in CI |
|---|---|---|---|---|
| `ambre-toile` | `logic::interop::pdf_export`, `logic::state::presentation::pdf_tests` | `--features pdf` | 8 | no |
| `ambre-server` | `redis_ops::lua_tests` | `--features test-helpers` | 6 | no |

Measured: `cargo test -p ambre-toile --lib` collects 341, with `--features pdf` 349 -- **the eight extra pass**. `cargo test -p ambre-server --features db` collects **156**, with `--features db,test-helpers` **162**; those six are Lua/Redis integration tests, so they are collected and compiled here but execute only against a live Redis (the CI `test-db` job's services), and this pass verified their existence and reachability, not their result.

The server case is the one that matters. Five of those six tests are the entire `Verification` cell of **`SRV-006`** -- "per `(document, client)` the `clock` MUST increase strictly, validated and assigned atomically in a single Redis round trip", the ordering rule the relay's anti-replay behaviour rests on. It was marked `verified` on the ZK pass on the strength of five tests that no job could compile. The first pass checked that every cited name resolved to a `fn`; it did not check that the `fn` was reachable, and two of the cited paths were wrong at the module level as well (`redis_ops::tests::` for tests living in `lua_tests` / `redis_integration_tests`).

Fixed in the `justfile`, which is where local/CI parity is defined: `test-native` gains `cargo test -p ambre-toile --features pdf`, `test-db` becomes `--features db,test-helpers`. **`Verification` now means "runs in CI", not "exists in the tree"** -- a name that resolves to a `fn` nobody executes is the same claim as a name that resolves to nothing.

### Remaining seeding order

1. ~~ZK boundary~~ -- done, above.
2. ~~Trame formula engine + CRDT convergence~~ -- done, above. The prediction was "a high `verified` ratio and few divergences": 52/61 `verified` across the four specs, and **twelve** divergences, none of them in the formula engine itself. They came from the surrounding spec -- an unreachable `#REF!` claim, an appendix outliving its cause, and a test list that had never been true. The grammar survived unchanged; what it needed was the `Sources` column pointing back at the paragraph that produced each rule, which is where every finding surfaced.
3. ~~The remaining app verticals~~ -- Toile done, above.
4. ~~Prose, ahead of its code~~ -- done: 76 rows across the four specs, every one `unimplemented` and bound to `ARCH P2`. This pass exists because the rule that forbade it was reversed (architecture §4): a design is reviewable as numbered falsifiable rules and is not reviewable as 5 800 lines of prose. What it produced immediately, beyond a readable contract: the same rule stated in two specs is now visibly the same rule (`PROSE-022` / `PROSE-FEAT-012` on local-only undo, `PROSE-009` / `PROSE-FEAT-017` on derived table geometry), and `prose-ui`'s 16 ms frame budget was deliberately **not** written as an invariant -- a number with no bench behind it cannot be falsified, which is the next item.
5. **The performance budgets** (architecture §5). Now the largest single gap and the next pass: numbered and falsifiable already, but verified by **one** criterion bench across the workspace for roughly thirty budgeted operations (architecture §5, `trame-logic §11`, `toile-logic §11`, `toile-ui §11`, `sync §4`). Every quality section in the corpus now ends on the same sentence -- the budgets cannot fail a build -- which is what makes it the next thing worth mechanising.
6. The foundation specs (`canvas`, `chrome`, `docfile`, `fixtures`), once the budgets have a mechanism.

One component end to end, then decide -- not every invariant in the corpus at once, which would be waterfall. Note what that rule is **not**: it is about the order of the passes, not about waiting for code. Writing a component's rules before its code is the normal case (`unimplemented`); writing every component's rules before any of them has been through the exercise is the mistake.

**Drift the ledger is meant to catch, observed on the first pass:** `security-audit.md` still lists `H5` (passphrases plaintext across IPC) as **OPEN**, while `ambre-bus/src/rest.rs` now carries `SecretBytes` / `SecretString` with `Zeroizing` + redacted `Debug`, and the module comment even cites `(H5)`. The finding was closed in code and the ledger was never updated. An ID joined to a `Verification` entry is exactly what makes that visible without re-reading the crate.

---

## 8. MECHANISM REALIZATION & GAPS

> What the ladder in architecture §4 prescribes, against what is actually wired (measured 2026-08). Listed so each absence is a decision rather than an omission. None of these blocks the current MVP.

| Mechanism | State | Gap |
|---|---|---|
| Type-level invariants | **strongest area** -- id newtype macro, `PublicKey`, `Ciphertext`, `SecretBytes`/`SecretString` | not the mechanism that is missing, the *statement*: nothing says which invariants these types hold, so the guarantee is invisible to anyone who has not read them |
| `#![forbid(unsafe_code)]` | 2 crates of 17 (`ambre-core`, `ambre-protocol`) | the pillar is claimed workspace-wide; measured zero `unsafe` anywhere, so lifting it to every crate is free and turns a claim into a mechanism |
| `[workspace.lints]` | absent | clippy runs the default set with `-D warnings`; no `pedantic`, no `deny(unwrap_used, panic, indexing_slicing)` |
| `cargo-deny` | **wired** (`deny.toml`, `just deny`, CI job `deny`) | advisories / licences / sources, plus the capability bans that mechanise `ZK-002`, `ZK-003`, `ZK-004`, `SRV-012`, `SYNC-011`. Negative-tested: re-declaring `chacha20poly1305` on `ambre-server` fails the check, naming that crate |
| `proptest` | wired | `ambre-crypto`, `ambre-trame::formula`, 2-peer convergence (`trame`, `toile`), `incremental_matches_rebuild` (toile index) |
| `cargo-fuzz` | absent | no target on postcard decode, the `.ambre` container, or the formula parser -- all fed from an untrusted relay or from user files |
| `cargo-mutants` | absent | ~1 170 test functions across the workspace, largely generated, never adversarially checked |
| `cargo-llvm-cov` | wired, non-blocking (`just cov`) | lcov baseline only; no diff-mode gate on added lines |
| `criterion` | 1 bench (`ambre-crypto/benches/crypto.rs`) | architecture §5 budgets roughly ten operations; no CI threshold job, so a regression cannot fail a build |
| `insta` | absent | candidates: `RecordingSurface` op sequences, wire encodings |
| `loom` / `miri` | **not adopted, by decision** | zero `unsafe`, atomics are flags and counters (`AtomicBool`/`AtomicU64`). Re-check with `rg 'unsafe|Ordering::' --stats` before revisiting |

### What the first `cargo deny` run turned up

Beyond the capability bans it was written for -- the reason a wall is worth having before anyone believes it works.

* **Four vulnerabilities fixed, one accepted.** `h2` was patched on our side of the tree (0.4.14 -> 0.4.16). The other three -- `h2` 0.3 and `rustls-webpki` 0.101 (`RUSTSEC-2026-0258`, `-0098`, `-0099`, `-0104`) -- came from `aws-sdk-s3`, whose default feature `rustls` is an alias for the SDK's **legacy** client stack (hyper 0.14 / rustls 0.21) even though the modern one (`default-https-client`, hyper 1 / rustls 0.23) was already being built alongside it. Declaring the SDK with `default-features = false` and the other four defaults verbatim drops the legacy path: `hyper` 0.14, `rustls` 0.21, `h2` 0.3 and `rustls-webpki` 0.101 all leave the graph. Only `RUSTSEC-2023-0071` (Marvin, `rsa` via `openidconnect`) stays accepted, with its justification and exit condition in `deny.toml`: the server holds no RSA private key and only verifies id_token signatures, and no fixed version exists upstream.
  * **Residual risk, stated:** nothing in the suite exercises a real S3 round trip (`ambre-spec-architecture.md §4` CI notes already record the missing minio service), so this TLS-stack swap is verified by compilation, by the dependency graph and by the 94 green host tests -- **not** by an actual handshake. A presigned upload against a local minio is the check to run before this reaches an environment that matters.

* **`unmaintained` is scoped to `workspace`.** A transitive unmaintained crate (loro's `im` / `bitmaps`, tooling's `paste`, `gumdrop`, `proc-macro-error2`) is upstream's decision, not a signal this repo can act on. Scoping it keeps the check readable instead of permanently red.
* **Ten crates declared no licence at all** (`ambre-core`, `ambre-crypto`, `ambre-protocol`, `ambre-bus`, `ambre-draw`, `ambre-pdf`, `ambre-trame`, `ambre-toile`, `ambre-prose`, `ambre-murmure`) while the other seven inherited `AGPL-3.0` from the workspace. Now inherited everywhere: an open-source suite whose crates half-declare their licence is a licensing defect, and it was found by a check written for something else entirely.
