# Ambre — Migration to Multi-threaded WASM with Shared Linear Memory

## Context

Ambre currently runs as a **3-Worker shared-nothing model**:

- **UI Thread**: Leptos chrome (planned; not yet integrated), wgpu/parley/swash/lyon
  rendering for the applicative area (already wired — Trame grid renders at 60+ Hz),
  DOM/canvas access, user input.
- **Logic Worker**: spreadsheet/document state, Loro CRDT, computation.
- **Sync Worker**: network I/O, server communication, conflict resolution.

In release builds, each worker is compiled to its own `.wasm` (three binaries —
`build_release.sh`). In dev builds, a single `ambre-app.wasm` is reused by three
Web Workers thanks to a dispatcher keyed on `DedicatedWorkerGlobalScope::name()`
(feature `unified`). Even in dev, each Worker instantiates its own
`WebAssembly.Memory`, so the heap is **not** shared — only the compiled module is.

Communication happens via `postMessage` + `postcard` serialization, abstracted
through `CommandSink`. The model is portable (no COI requirement) but pays a
visible postMessage round-trip latency cost — observable at 60fps when the UI
Thread requests spreadsheet ranges not cached locally from the Logic Worker.

**Target**: a single WASM binary with `std::thread` / rayon-style threading,
shared linear memory, and native Rust channels between threads. Web Workers
remain under the hood as execution primitive only — invisible at the Rust API
level.

## Architectural shift

- All threads share the same `WebAssembly.Memory` (backed by
  `SharedArrayBuffer`). The entire Rust heap is visible across threads.
- Native synchronization primitives work: `Arc<T>`, `Arc<RwLock<T>>`,
  `flume::channel`, `std::sync::mpsc`, `tokio::sync::mpsc` (in single-threaded
  async mode).
- The main thread remains special: it is the **only** thread that can touch the
  DOM, canvas elements (and therefore wgpu surfaces created from them), and DOM
  event listeners.
- Main thread cannot call `Atomics.wait` (would block the event loop, forbidden
  by the spec). It must use `Atomics.waitAsync` or polling. Wrapped naturally
  via `wasm-bindgen-futures::spawn_local`.
- Compute threads (Logic, Sync) can block on channels normally.

## Cross-Origin Isolation (mandatory)

Set on **all** HTML responses serving the app:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: credentialless
```

This enables `crossOriginIsolated === true` in the document, which unlocks
`SharedArrayBuffer` and therefore multi-threaded WASM. Use `credentialless`
rather than `require-corp` — it strips credentials from cross-origin
sub-resource requests instead of requiring each third party to opt in via CORP
headers. Much less invasive.

## Per-concern checklist

> **Note.** As of the start of this migration, almost none of the features below
> are wired in the client (no SSO, no Object Storage client, no Blind Relay, no
> third-party embeds, no Google/MS import-export). The items in this section are
> therefore **writing rules to apply when each feature is implemented**, not
> migration work on existing code. They stay here so the constraints are not
> forgotten at the moment each feature lands.

### SSO / Authentication

- **Drop** popup-based OAuth/OIDC flows. `COOP: same-origin` breaks
  communication with the popup.
- **Drop** silent auth via hidden iframe (`prompt=none`). With `credentialless`
  the iframe receives no session cookies from the IdP.
- **Adopt** Authorization Code Flow with redirect:
  - Full page navigation to the IdP, callback to a URL on Ambre's domain.
  - Persist app state in IndexedDB before redirect, rehydrate on callback.
- Applies to Open Buro OIDC, future SAML, and any third-party OAuth (Google,
  Microsoft) used for import/export integrations.
- `COOP: restrict-properties` would allow popups but is Chromium-only — not
  viable for a multi-browser office suite.

### Object Storage (Scaleway, presigned URLs)

- Configure CORS on each bucket holding cipher blobs:
  ```
  AllowedOrigins:  ["https://app.ambre.eu"]
  AllowedMethods:  ["GET"]
  AllowedHeaders:  ["*"]
  ExposeHeaders:   ["ETag", "Content-Length"]
  ```
- Load bitmaps via `<img crossorigin="anonymous" src="...">` or
  `fetch(url, { mode: 'cors', credentials: 'omit' })` followed by
  `createImageBitmap(blob)`.
- The presigned signature is in the query string, not in credentials — it
  survives `credentialless`.
- Validate end-to-end in staging before going wide.

**Architectural note**: presigned URLs leak object identifiers via query
strings (metadata leak), defeat HTTP caching (URL changes per generation), and
couple us to provider-specific signature formats. Consider routing blob
fetches through the Blind Relay with hash-based immutable URLs as an
alternative, even though it costs server bandwidth.

### Blind Relay (own backend)

If served cross-origin to the app (e.g., `api.ambre.eu` vs `app.ambre.eu`),
set on responses:

```
Cross-Origin-Resource-Policy: same-site
Access-Control-Allow-Origin: https://app.ambre.eu
Access-Control-Allow-Credentials: true   # if needed
Access-Control-Allow-Methods: GET, POST, PUT, DELETE
Access-Control-Allow-Headers: Content-Type, Authorization, ...
```

If served same-origin (same domain, sub-path), no CORS is needed.

### Third-party embeds (YouTube, Vimeo, SoundCloud, etc.)

- Use `<iframe credentialless src="https://www.youtube.com/embed/...">`.
- Browser support: Chrome 110+, Firefox recent stable, Safari partial — verify
  current state at integration time.
- The embed loads without credentials: video playback works, but no
  logged-in features (history, premium, save-to-playlist).
- Acceptable for ≥99% of embed use cases in spreadsheets and documents.
- Provide a documented fallback for Safari if `credentialless` is not yet
  supported in the user's version (e.g., open in new tab instead of inline
  embed).

### Google Workspace / MS Office import-export

- **All flows are client-side.** Never proxy import/export through the Blind
  Relay — it must remain inside the zero-knowledge perimeter.
- Flow:
  1. OAuth redirect to Google/Microsoft → token returned to Ambre.
  2. Client `fetch()` to `googleapis.com` / `graph.microsoft.com` REST APIs
     with `Authorization: Bearer <token>`. The bearer header is not stripped
     by `credentialless` (it strips only browser-managed credentials).
  3. Parse XLSX/DOCX/PPTX in Rust/WASM (rust-xlsx, calamine, docx-rs, etc.).
  4. Encrypt locally and store in Ambre.
- **E2EE caveat**: exported copies live in cleartext at the third-party
  provider. This is inherent to exporting to a non-E2EE service, not a flaw of
  Ambre. Surface this clearly in the UI before any export.
- Recommend an explicit opt-in toggle "Enable external cloud integrations",
  off by default, with a clear warning about implications.

### Other external resources to audit

- **Web fonts**: self-host. Sovereignty + cacheability + COI compliance for free.
- **JS polyfills / libs from CDN**: self-host.
- **Telemetry (Sentry, etc.)**: self-host the endpoint or use an EU-hosted
  provider with proper CORS.
- **Map tiles** (if used in cell content): OpenStreetMap or an EU tile provider
  with CORS configured.
- **WebRTC for live collaboration**: unaffected by COI. Signaling goes through
  the Blind Relay; data channels are peer-to-peer.
- **Webhooks to/from external services** (Slack notifications, etc.): handled
  server-side, not affected by client COI.

## Toolchain

Validated end-to-end in Phase 1 of this migration. The exact recipe that works:

- Rust target: `wasm32-unknown-unknown`, **nightly required** to rebuild `std`
  with atomics. Pin a specific nightly date in `rust-toolchain.toml` for
  reproducible builds; bump deliberately.
- Threading lib: [`wasm_thread`](https://crates.io/crates/wasm_thread) — drop-in
  `std::thread::spawn` replacement that creates a Web Worker per thread under
  the hood. (`wasm-bindgen-rayon` is an alternative for data-parallel pools;
  we don't need it for the 2-thread logic/sync model.)
- `.cargo/config.toml`:
  ```toml
  [target.wasm32-unknown-unknown]
  rustflags = [
      "-C", "target-feature=+atomics,+bulk-memory,+mutable-globals",
      # LLD: emit a shared imported memory + export the TLS helpers
      # wasm-bindgen's threads transform needs.
      "-C", "link-arg=--shared-memory",
      "-C", "link-arg=--max-memory=4294967296",
      "-C", "link-arg=--import-memory",
      "-C", "link-arg=--export=__wasm_init_tls",
      "-C", "link-arg=--export=__tls_size",
      "-C", "link-arg=--export=__tls_align",
      "-C", "link-arg=--export=__tls_base",
  ]
  [unstable]
  build-std = ["std", "panic_abort"]
  ```
- Why each link-arg matters (each was a step in the Phase 1 debug):
  - **Without `--shared-memory`**: memory is not `shared`, `postMessage(memory)`
    to the worker fails with `DataCloneError`.
  - **Without `--import-memory`**: wasm-bindgen's threads transform panics with
    `assertion failed: mem.import.is_some()` — it requires the memory to be
    imported (JS creates it, hands it to every worker).
  - **Without the four `--export=__*` flags**: wasm-bindgen fails with
    `failed to find __wasm_init_tls` — these helpers are generated by LLD but
    not exported by name unless asked.
- `wasm-bindgen-cli` version must match the `wasm-bindgen` crate version.
  Trunk handles this automatically. Validated with 0.2.117.
- Dependency audit (Phase 1 result): all current workspace deps compile with
  `+atomics`. Notably **Loro, parley, swash, harfrust, wgpu, lyon, taffy,
  gloo-net, getrandom** — none required forks or patches.

## Architectural impacts

### Binary unification

Three binaries collapse into one. Expected download-size reduction in the
30–40% range relative to the sum of the three, because the Rust runtime
(allocator, panic handler, `String`/`Vec` monomorphizations, `Display`/`Debug`
glue) and shared dependencies (postcard, serde, Loro) are no longer
duplicated.

Code-splitting for per-app loading (Trame / Prose / Toile) becomes a separate
concern, addressable later via `wasm-split` or manual dynamic imports if
download size becomes a bottleneck again.

### Loss of compile-time isolation

Today, importing UI code into the Logic Worker is impossible — the crate isn't
in scope. After unification, this discipline must be enforced via:

- Crate workspace structure: `ambre-ui`, `ambre-logic`, `ambre-sync` as
  separate crates inside the workspace, with the binary crate as a thin entry
  point that wires them together and routes threads.
- Visibility rules: `pub(crate)` everywhere, narrow re-exports.
- Convention: a thread's `main` function only imports from its own crate.
- Linting / code review for cross-module access on the hot path.

### Communication patterns

- `CommandSink` abstraction: retire or reduce to a thin wrapper over a channel.
- Postcard serialization on the inter-thread path: remove. Keep postcard only
  where serialization is genuinely needed (disk persistence, wire format for
  the Blind Relay).
- Loro state can live in `Arc<RwLock<LoroDoc>>` accessible from Logic and Sync
  threads. UI thread reads through a channel-mediated snapshot or via async
  futures awaiting `Atomics.waitAsync`.
- Main thread async tasks: wrap blocking primitives in
  `wasm_bindgen_futures::JsFuture` so the event loop stays responsive.

### Memory model

- Hard 4 GiB cap on `wasm32` linear memory. All threads share it.
- Plan total memory budget across all concurrent sub-apps (a user with Trame +
  Prose + Toile open shares one 4 GiB heap).
- `Arc` reference counting performs atomic ops on every clone/drop; in shared
  memory these are slightly more expensive than native. Avoid hot `Arc` clones
  in tight loops.
- `wasm64` is emerging but not production-ready as of 2026.

## Caveats and trade-offs

- **Hard dependency on COI**: Ambre cannot run in any context where COOP/COEP
  cannot be set on the document response. Rules out embedding Ambre inside
  an iframe of a host that doesn't cooperate (LMS, intranet portals, etc.)
  unless that host itself is COI-compliant.
- **`<iframe credentialless>` support is partial in Safari**. Have a fallback
  strategy for users on older Safari versions.
- **Toolchain complexity** increases noticeably: custom RUSTFLAGS, possibly
  nightly, dependency audits, more complex CI setup.
- **Lost portability**: in current model A, Ambre would work without COI (just
  slower). In model B, no COI means no Ambre.

## Migration phases

Each phase is a self-contained chunk of work that leaves the codebase in a
buildable, runnable state. Periphery work (SSO, Object Storage, embeds,
import-export) is deferred to "writing rules" — see the
`Per-concern checklist` note above — and is **not** part of these phases.

### Phase 0 — Spec & precondition update *(✅ done)*

- Acknowledge actual starting state (Leptos planned but not yet integrated,
  `unified` dev mode reuses one `.wasm` across three Workers with three
  separate `WebAssembly.Memory` instances).
- Mark periphery checklist items as writing rules for the moment each
  feature is implemented, not migration steps.

### Phase 1 — Toolchain spike *(✅ done)*

Goal: prove the combination `nightly + build-std + atomics + wasm_thread +
all current workspace deps` produces a working multi-threaded WASM binary
with end-to-end shared memory between main thread and a spawned worker.

Outcome:
- All dependencies compile with `+atomics` (including Loro — the #1 risk).
- `crossOriginIsolated === true` honored under Trunk's `[serve.headers]`.
- `wasm_thread::Builder::spawn` works end-to-end; a worker thread mutates an
  `Arc<AtomicUsize>` and the main thread observes the increments via rAF
  polling — proving the heap is genuinely shared.
- The exact set of linker flags required by wasm-bindgen's threads transform
  is captured in the `Toolchain` section above.

### Phase 2 — Workspace restructure *(next)*

Compile-only change. Runtime behavior unchanged.

- Merge `ambre-sync-worker` (currently empty-ish cdylib) into the existing
  `ambre-sync` rlib. Public surface becomes `pub fn run(channels…)`, no more
  `#[wasm_bindgen(start)]`, no more cdylib.
- Rename `ambre-logic-worker` → `ambre-logic`. Same shape: rlib only, public
  `pub fn run(channels…)`.
- Delete `build_release.sh`, `build_workers_dev.sh`,
  `trunk_serve_standalone.sh`, `Trunk.standalone.toml`, the
  `logic_worker_*.js` / `sync_worker_*.js` bootstraps, and the `unified` /
  `standalone` Cargo features.
- `ambre-app/Cargo.toml`: depend on `ambre-logic` and `ambre-sync` directly,
  no `optional`, no features.
- `index.html`: drop the worker `copy-file` entries.

### Phase 3 — IPC refactor (the core)

Replace `postMessage + postcard` with native Rust channels.

- Add `flume` to workspace deps. Add a `SendCommandSink` impl of
  `CommandSink` over `flume::Sender<SyncCommand>` in `ambre-bus`.
- `ambre-app/src/lib.rs`:
  - Allocate 4 `flume::unbounded()`:
    `(ui_to_logic, logic_to_ui, logic_to_sync, sync_to_logic)`.
  - `wasm_thread::spawn(move || ambre_logic::run(rx_ui, tx_ui, tx_sync, rx_sync))`.
  - Same for `ambre_sync::run`.
  - Drop `Worker::new_with_options`, `MessageChannel`, the `set_onmessage`
    handlers, and the bootstrap handshake closures.
  - rAF loop: `try_iter()` non-blocking on `logic_to_ui_rx` instead of the
    `Rc<RefCell<VecDeque<LogicEnvelope>>>` fed by `onmessage`.
  - Keyboard input emits `ui_to_logic_tx.send(UiEnvelope::AppCommand{…})`
    directly — no postcard, no ArrayBuffer copy.
- `ambre-logic::run`, `ambre-sync::run`: blocking loops on their inbound
  channels. Each `AppController` keeps its current `Vec<u8>` payload shape
  for now (typing the inter-app payloads is a separate future refactor).
- Strip `postcard` from every inter-thread path. Keep it only on disk
  persistence and the Blind Relay wire format.

### Phase 4 — Runtime validation

- Existing IPC smoke-test must keep passing, with the round-trip latency
  budget dropping from ~1–17 ms (rAF-aligned) to **< 100 µs**.
- Viewport scroll benchmark with cells computed live (not cached) must
  hold a stable 60+ FPS — this is the headline reason for the migration.
- DOM-access audit: `rg 'web_sys::|HtmlCanvasElement|wgpu::' crates/ambre-{logic,sync}/`
  must return **zero results**. Leptos chrome, when introduced, sits next to
  `ambre-app` (main thread) only.

### Phases deliberately deferred (writing rules, see checklist)

When SSO, Object Storage, Blind Relay, embeds, or external import-export
are first implemented, they must respect the constraints in the
`Per-concern checklist`. There is no "migration" of existing code for these
because no client code for any of them exists today.

## Reference: model comparison

| Aspect                       | Current (Model A)           | Target (Model B)              |
| ---------------------------- | --------------------------- | ----------------------------- |
| Number of WASM binaries      | 3                           | 1                             |
| Memory model                 | Per-worker isolated heaps   | Shared linear memory          |
| Inter-thread comm            | `postMessage` + postcard    | Native Rust channels          |
| Round-trip latency           | ~ms                         | ~µs                           |
| Cross-origin isolation       | Not required                | Required                      |
| Toolchain                    | Standard `wasm32`           | Atomics + threading flags     |
| Compile-time isolation       | Strong (separate crates)    | Convention-based              |
| Portability                  | Any browser, any embedding  | COI-capable contexts only     |
| Estimated download size      | ~13 MB (sum of three)       | ~8 MB (single, deduped)       |

## Open questions

Resolved by Phase 1:
- ✅ Loro client compiles cleanly with `+atomics`. No fork, no patch.
- ✅ wgpu compiles cleanly with `+atomics`. Leptos integration to be
  re-verified when the chrome lands, but no toolchain-level blocker
  expected (Leptos has no atomics-unfriendly internals).

Still open, to be addressed at the appropriate phase or feature:
- Validate `<iframe credentialless>` on current Safari stable across target
  user platforms — at the time the first third-party embed is wired.
- Decide whether blob fetches stay on presigned URLs or move to Blind Relay
  proxying — independent of this migration; revisit when Object Storage is
  first wired.
- Decide the memory budget per sub-app to avoid hitting the 4 GiB ceiling
  with several apps open simultaneously — Phase 3 or later, once real
  allocation patterns are observable.
