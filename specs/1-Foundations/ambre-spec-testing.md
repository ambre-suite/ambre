# AMBRE — SPEC: Test strategy realization & coverage debt
**Type:** Horizontal (Foundation — Quality)
**Status:** Draft

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
