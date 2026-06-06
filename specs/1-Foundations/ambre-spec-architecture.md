# AMBRE — SPEC: GENERAL ARCHITECTURE
**Type:** Horizontal (Foundation)
**Status:** Draft

> Technical architecture of AMBRE: **3-thread shared-memory model** (single WASM binary, `wasm_thread` + `SharedArrayBuffer`), crate breakdown and dependency boundaries, **typed** inter-thread IPC transport (`flume` channels, Rust values end-to-end), three-plane data plane (content / service / network), zero-knowledge security, performance budgets, and integration roadmap.

---

## 1. OBJECTIVES & CONSTRAINTS

* **Scope:** This spec defines AMBRE's client-server technical architecture: Rust workspace structure, responsibilities of each crate, data flow between the three threads, IPC transport, and dependency rules. It does not describe the internal implementation of components (those details are in specific specs like `ambre-spec-sync.md`).

> **Model note (read first).** The first version of this spec described **Model A**: 3 separate WASM binaries (`shared-nothing`), communication via `postMessage` + `ArrayBuffer` transfer + `postcard` serialization. **The actual code is Model B**: **a single WASM binary**, three `wasm_thread` threads sharing a **linear memory** (`SharedArrayBuffer`), communicating through **`flume` channels** carrying **typed Rust values** end-to-end. All historical justifications like "such-and-such heavy dependency must not pollute such-and-such binary" are moot — there is only one binary. **Boundaries are now enforced by the crate dependency graph** (verified by the compiler), not by binary separation. Model A and its corollaries (`postMessage`, separate JS Web Workers) are recorded in **§6** so they are not re-proposed.

* **Dependencies (global rules):**

| Rule | Rationale |
|-------|---------------|
| `ambre-server` depends ONLY on `ambre-core` + `ambre-protocol` + `ed25519-dalek` (direct) | The Blind Relay must not know document structures or application crypto types (`DocumentKey`, `MasterSeed`). The server's only crypto operation is Ed25519 signature verification on updates — `ed25519-dalek` is pulled directly, bypassing `ambre-crypto`. |
| `ambre-bus` depends on `ambre-core` **only** | The 4 IPC envelopes travel as **Rust values over `flume`** — nothing is serialized. No more `serde`/`postcard`/`thiserror`/`flume`/`tracing` dependencies. Application payloads are **typed** (`AppCommand`/`AppEvent`); only real network bytes (Loro CRDT deltas, channel messages) remain `Vec<u8>`. This upstream leaf guarantees `ambre-bus` stays free of crypto/network dependencies. |
| `ambre-draw` depends on `kurbo`, `peniko`, `parley` only (no `wgpu`, no `krilla`) | `DrawSurface` trait and re-exports of geometric/typographic types. Light type dependencies (math, colors, text layout) — no GPU dependency. Concrete implementations (`WgpuSurface` on the UI side, `KrillaSurface` on the Logic side) live in consumer crates. |
| `ambre-commands` depends on **nothing UI-side** (leaf; `ambre-core` for ids/`Locale` as needed) | Shared command registry: `CommandId`, `Command`, `CommandState`, `CommandSource`, keybindings. Depended on by **every app crate** (which defines its commands) AND by `ambre-chrome` (which renders them). Distinct from `AppCommand`/`LogicCommand` IPC from `ambre-bus`: an invoked `Command` *produces* a `LogicCommand`. |
| `ambre-canvas` depends on `ambre-draw` (+ wgpu/swash + accesskit ; + web-sys/js-sys/wasm-bindgen(-futures)/tracing as **`cfg(wasm32)`-targeted** deps for `host`) | **Canvas render runtime** of the UI thread: `WgpuSurface`/`QuadRenderer`/`GlyphAtlas`, `TextCache`, `FrameHost`/`DocView`, generated `theme.rs`. **Does NOT depend on `leptos`.** `accesskit` is scoped to canvas **content** a11y. **No application dependency** (coupling via `DocView` + closure injection). Named `ambre-canvas` (medium, symmetrical to `ambre-chrome`); spec `ambre-spec-canvas.md`. |
| `ambre-chrome` depends on `leptos` + `ambre-commands` | **Shared, app-agnostic Leptos chrome toolkit**: `AppShell`, `CommandPalette`, `Dialog`/`Menu`/`Toolbar`/`Toast`/`ContextMenu`/`Sidebar`/`StatusBar` primitives, keybinding manager. Renders views over the registry — **names no app** (same inversion as `DocView`). App-**specific** chrome lives in the app crate, **not here**. |
| Each app crate (`ambre-trame`, ...) depends on `ambre-bus` + `ambre-draw` + `ambre-canvas` + `ambre-chrome` + `ambre-commands` + `leptos` (NOT `ambre-sync` or `ambre-logic`) | One crate per app, **deliberate monolith**, **no features**: `logic` (`*State` + handlers, Logic thread) + `ui` (`*View` impl `DocView`, content rendering on `ambre-canvas`) + `chrome` (**specific** Leptos components built on `ambre-chrome`) + `Command` definition (`CommandSource`). IPC types (`TrameCommand`/`TrameEvent`/`DisplayCell`) re-exported from `ambre-bus`. |
| `ambre-logic` depends on `ambre-bus` + the app crate(s) + `flume` + `tracing` | Logic thread runtime. **100% pure Rust, zero WASM glue**: fundamental types arrive via `ambre-bus` re-exports (no direct dep on `ambre-core`). The panic hook + tracing subscriber are set up in `ambre-app`. |
| `ambre-sync` depends on `ambre-bus` + `ambre-protocol` + `flume` + `tracing` + `wasm-bindgen` + `web-sys` (+ `zeroize`/`gloo-net`/`ruzstd` placeholders, flagged) | Sync thread runtime. Wire encoding comes from `ambre-protocol`; crypto will come from `ambre-crypto`. |
| `ambre-app` depends on `ambre-bus` + `ambre-canvas` + `ambre-chrome` + `ambre-trame` + `ambre-logic` + `ambre-sync` + `flume` + `wasm-bindgen` + `wasm_thread` | cdylib LAUNCHER. **Mounts the Leptos `AppShell`** (`ambre-chrome`), creates the 4 channels, spawns `ambre_logic::run` + `ambre_sync::run`, bootstraps the canvas and injects the active app's content view. Does NOT depend on wgpu/postcard/ambre-core directly. |

* **Invariants:**
- **No dependency cycles**: the graph is acyclic and verifiable via `cargo tree`. It is the graph that carries the architecture boundaries (no longer binary separation).
- **The Logic thread touches neither GPU nor DOM**: `ambre-logic` has **no direct dependency** on `ambre-canvas`/`ambre-chrome`, and its **runtime path** never calls wgpu/leptos. (Transitively, `ambre-logic → app crate → ambre-canvas/ambre-chrome` makes wgpu/leptos *reachable* in the graph — without consequence: single binary, the boundary is carried by **direct deps + runtime**, not by the transitive graph.) The Sync thread compiles neither UI nor CRDT as direct deps.
- **Golden rule — Strict separation of concerns between threads:**
  - **UI Thread** (page's main thread): rendering (wgpu + QuadRenderer + GlyphAtlas + Parley/Swash), input capture, accessibility (AccessKit). **Zero API calls**, **zero WebSocket**, **zero crypto**, **zero compression**. Data arrives already display-ready (typed `AppEvent` values). Sole exception: **font** loading (static assets, same-origin). **New invariant: the UI thread must NEVER block** (it runs on the page's main thread).
  - **Logic Thread**: business logic (Loro CRDT, pagination, Trame formula engine, typed dispatch). **Zero network**, **zero crypto**, **zero compression**. No `DocumentKey`, `ciphertext`, `WsClientMsg`, `WebSocket` types in app crates. `SyncCommand` only carries `document_id` and raw CRDT payloads (`Vec<u8>`).
  - **Sync Thread**: **all** networking (HTTP, WebSocket, S3 presigned URLs), **all** crypto (`ambre-crypto`), **all** compression (`ruzstd`). Sole consumer of `ambre-crypto` and `ambre-protocol` client-side.
- **`ambre-protocol` is the single source of truth** for all network protocol types. Neither `ambre-sync` nor `ambre-server` redefine them.
- **The server never sees plaintext**: all types transiting through `ambre-protocol` are opaque blocks (ciphertext + public metadata).
- **postcard stays on the network path only** (`ambre-protocol`, WebSocket wire). **The inter-thread path carries no postcard**: `flume` envelopes travel as typed Rust values. JSON only for the REST API.
- **Toolchain:** pinned nightly + `-Z build-std` (rebuilding `std` with `+atomics`), `target-feature=+atomics,+bulk-memory,+mutable-globals`, link-args `--shared-memory/--max-memory/--import-memory` + TLS exports. See `rust-toolchain.toml` + `.cargo/config.toml`.

---

## 2. TECHNICAL DESIGN & STATE

### A. Workspace Structure

```
ambre/
├── Cargo.toml                    (root workspace — members below)
├── rust-toolchain.toml           (pinned nightly, rust-src, wasm32 target)
├── .cargo/config.toml            (wasm32 rustflags: +atomics, --shared-memory, ..., build-std)
├── crates/
│   ├── ambre-core/               Fundamental types (Uuid/DocumentId/UserId/..., DocType, PublicKey, Role). Shared client + server.
│   ├── ambre-crypto/             Crypto primitives. ⚠️ STUB (lib.rs ~6 lines, no real consumer).
│   ├── ambre-protocol/           Network types (Update/Snapshot/Ephemeral, Ws*Msg, Signaling*, encode/decode). SOURCE OF TRUTH (sync + server).
│   ├── ambre-bus/                Client IPC contract. Depends on ambre-core ONLY. No serde/postcard/flume/tracing.
│   │     ├── lib.rs              (re-exports + IPC_TEST_DOC_ID)
│   │     ├── logic.rs            (LogicCommand + AppCommand + TrameCommand)
│   │     ├── ui.rs               (UiEvent + AppEvent + TrameEvent + DisplayCell)
│   │     ├── command.rs          (SyncCommand)
│   │     ├── event.rs            (LogicEvent)
│   │     └── rest.rs             (RestCommand + RestEvent + RequestId + shared business types)
│   ├── ambre-draw/               DrawSurface trait + kurbo/peniko/parley re-exports. No GPU dep.
│   ├── ambre-commands/           Shared command registry (CommandId/Command/CommandState/CommandSource/keybindings). Leaf, zero UI dep.
│   ├── ambre-canvas/                 Canvas render runtime of the UI thread (NO leptos):
│   │     ├── render/             (QuadRenderer, GlyphAtlas, WgpuSurface, 2-layer Layer model)
│   │     ├── text_cache.rs       (TextCache Parley)
│   │     ├── view.rs             (DocView trait + RepaintPlan + FrameStats + InputEvent)        [GPU-agnostic]
│   │     ├── host.rs             (FrameHost<V> + GpuState + bootstrap_gpu + start_raf_loop + run) [cfg(wasm32)]
│   │     └── theme.rs            (canvas-side constant tokens, generated from tokens.ron ; accesskit scoped to content)
│   ├── ambre-chrome/             Shared, app-agnostic Leptos chrome toolkit: AppShell, CommandPalette,
│   │                             Dialog/Menu/Toolbar/Toast/ContextMenu/Sidebar/StatusBar, keybindings.
│   │                             Depends on leptos + ambre-commands. App-SPECIFIC chrome lives in app crates.
│   ├── ambre-trame/              ALL of the Trame app (monolith, no features):
│   │     ├── logic.rs            (TrameState + handlers, Logic thread)
│   │     ├── ui/{grid,renderer,view}  (TrameView impl DocView, content rendering on ambre-canvas)
│   │     └── chrome/             (SPECIFIC Leptos components on ambre-chrome ; Command via ambre-commands)
│   ├── ambre-prose / ambre-toile / ambre-murmure   Stubs (doc only, no logic).
│   ├── ambre-logic/              Logic thread runtime: run(from_ui, to_ui, to_sync, from_sync),
│   │                             LogicState { docs: HashMap<Uuid, DocState>, ... }, match dispatch. 100% pure Rust.
│   ├── ambre-sync/               Sync thread runtime: run(from_logic, to_logic), WS/crypto pipeline (placeholder).
│   ├── ambre-app/                cdylib LAUNCHER: #[wasm_bindgen(start)], mounts Leptos AppShell (ambre-chrome),
│   │                             creates the 4 channels, spawns logic + sync, bootstraps the canvas + injects active view.
│   └── ambre-server/             Native Blind Relay (Axum). OUTSIDE workspace members (native target, not wasm32).
└── specs/
```

> **`ambre-server` is in the root workspace `members`**: the server is a native target (Axum/Tokio) while other crates target `wasm32-unknown-unknown`, but rustflags (`+atomics`, `--shared-memory`) are scoped by target (`[target.wasm32-unknown-unknown]`) and do not affect native builds. `build-std` is not implicit: it only activates when `-Z build-std` is passed explicitly. Integrating the server in the workspace simplifies dependency management (shared `workspace = true`), tooling (unified `cargo fmt`, `cargo clippy`, `cargo check`), and CI (single workspace).

> **Build workflow:** a single cdylib build (`ambre-app`). The Model A `*-worker` crates (`ambre-logic-worker`, `ambre-sync-worker`) **no longer exist**; no more `make workers` or separate wasm-pack step. Trunk (or equivalent) builds `ambre-app` and copies assets (`fonts/`).

### B. Boundaries, Data Flow, and 3-thread Model (shared memory)

The client architecture is organized into **3 `wasm_thread` threads** within a **single WASM module**, sharing a **single linear memory** (`WebAssembly.Memory` `shared`, backed by `SharedArrayBuffer`). `wasm_thread::spawn` instantiates a Web Worker under the hood, but all load **the same module** and **import the same memory** — hence the common heap. Inter-thread communication goes through **`flume` channels** carrying **typed Rust values by move** (zero serialization, zero boundary copy):

```
┌─────────────────────────────────────────────────────────────────┐
│  UI THREAD (page's main thread)                                   │
│  Chrome: Leptos DOM (ambre-chrome) — AppShell/menus/dialogs/...    │
│  Content: ambre_canvas::run → FrameHost<V> in the <canvas>             │
│  wgpu (QuadRenderer / GlyphAtlas / WgpuSurface), Parley/Swash     │
│  HTMLCanvasElement, GPUDevice — !Send, confined here              │
│  requestAnimationFrame → DocView::{update, plan_repaint, paint*}  │
│  ⚠️ never BLOCKS (try_send / try_recv on the UI side)             │
└───────────────┬──────────────────────────────▲──────────────────┘
   flume bounded │ LogicCommand (UI → Logic)    │ UiEvent (Logic → UI)
                 ▼                              │
┌───────────────────────────────────────────────────────────────────┐
│  LOGIC THREAD (wasm_thread — shared heap)                          │
│  ambre_logic::run → LogicState { docs: HashMap<Uuid, DocState> }    │
│  ambre-trame (TrameState) — LoroDoc, match dispatch                 │
│  Selector::wait on (from_ui, from_sync)                            │
└───────────────┬──────────────────────────────▲──────────────────┘
   flume bounded │ SyncCommand (Logic → Sync)   │ LogicEvent (Sync → Logic)
                 ▼                              │
┌───────────────────────────────────────────────────────────────────┐
│  SYNC THREAD (wasm_thread — shared heap)                           │
│  ambre_sync::run → SyncState                                       │
│  web_sys::WebSocket — !Send, confined here                        │
│  Crypto / compression pipeline (placeholder)                      │
└───────────────┬───────────────────────────────────────────────────┘
                │  Binary WebSocket  ([version_byte] [postcard(WsServerMsg)])
                │  ambre-protocol : WsClientMsg / WsServerMsg
┌───────────────▼───────────────────────────────────────────────────┐
│  BLIND RELAY (ambre-server — native, Axum/Tokio)                   │
│  Dependencies: ambre-core + ambre-protocol + ed25519-dalek         │
└───────────────────────────────────────────────────────────────────┘
```

**Symmetry of the three runtimes:**
```
ambre_canvas::run(canvas_id, make_view)                     ← UI thread (generic over DocView)
ambre_logic::run(from_ui, to_ui, to_sync, from_sync)    ← Logic thread
ambre_sync::run(from_logic, to_logic)                   ← Sync thread
```
The only app-aware piece on the UI side (constructing `TrameView`) is **injected via closure** from `ambre-app` → no cycle (`ambre-canvas` does not depend on `ambre-trame`).

**Rationale for Logic/Sync separation:** decryption + decompression of a remote update (~0.5–2 ms on mobile) must not interrupt the Logic thread's response to P1 events (scroll). With two separate threads and a shared heap, these operations execute in **true parallelism without payload recopy**: a `Vec<u8>` moves from one thread to another **by move** on the `flume` channel, with no boundary memcpy.

**Rationale for the shared-memory model (vs shared-nothing — §6):** `flume` transport moves typed Rust values (including application payloads) with no serialization or recopy — in contrast to `postMessage` in Model A which imposed `postcard` + two WASM↔JS memcpy per hop. Trade-off: the **toolchain** (nightly + `-Z build-std` + `+atomics` flags) and the **HTTP COOP/COEP headers** required by `SharedArrayBuffer`. Separation of concerns remains guaranteed — no longer by binary memory isolation, but by the **crate dependency graph**.

**Backpressure — bounded channels.** The 4 `flume` channels are `bounded(CHANNEL_CAPACITY = 1024)` (created in `ambre_app::main`). An unbounded channel would let a momentarily slow consumer accumulate an unbounded queue in the shared heap (e.g., scroll key-repeat stacking `ScrollViewport` faster than Logic drains). **Worker thread senders (Logic, Sync) may block on a full channel** = this is the intended backpressure. **The UI thread never blocks**: `TrameView::send_command` uses `try_send` and **drops on `Full`** (a viewport request is superseded by the next one via `version`; no non-idempotent command transits this hot path today). Additionally, `ScrollViewport` are **coalesced to ≤ 1 send per frame** (`handle_input` accumulates in `pending_viewport`, `update` flushes once per frame).

> ⚠️ **Known limitation (`Atomics.wait`).** Blocking on `flume::Selector::wait` / `recv` in a WASM Worker uses `Atomics.wait`, which **freezes the Worker's JS event loop** (WebSocket/`setTimeout`). This is legal **outside** the main thread (Logic and Sync are workers). OK as long as the WS pipeline is a placeholder; **when the real WebSocket is wired, the Sync thread will need to switch to a non-blocking loop** (`recv_async` + WS driven by `JsFuture`/`spawn_local`). See §7 ARCH P1.

**"Thread failure" model.** Two paths:
- **Graceful**: a thread detecting an unrecoverable condition emits `UiEvent::FatalError { message }` **before** exiting. `TrameView::update` then switches to `fatal` state (no more sends, single log).
- **Non-graceful (panic)**: under WASM multi-thread, `panic=abort` (via `build-std` + `panic_abort`). ⚠️ **Correction of a previous assertion:** `panic=abort` **does not unwind the stack**, so **no `Drop` runs** and **no `Sender` is dropped** — the Logic thread's `Sender<UiEvent>` lives in **shared** memory and is **leaked** on abort. The UI therefore sees `Empty` forever, **never `Disconnected`**: we **cannot** rely on channel disconnection to detect a panic. Detection must go through an **out-of-band** signal: `panic::set_hook` (already set in `ambre-app`) must, in addition to logging, call a JS hook `ambreFatal(message)` (symmetrical to `ambreReady`) **before** the abort. *Still to be wired (§7 ARCH P1).* Channel disconnection is, at best, only a **clean shutdown** signal (voluntary teardown).

**Per-tab isolation:** each tab instantiates its own complete application (3 threads + 1 shared heap). Two tabs opening the same document are two independent CRDT clients — convergence via the Blind Relay (same flow as two users). Loro handles this natively. Post-MVP optimization: inter-tab `BroadcastChannel`.

**Multi-document sync:** the Sync thread manages **N sync sessions in parallel** (`HashMap<DocumentId, SyncDocumentState>`) over a **single multiplexed WebSocket**. Required for embedded documents (a Toile can include a Prose + a Trame, each with its `DocumentKey` and `LoroDoc`). Demultiplexing by `document_id`. `OpenDocument` → `Subscribe`, `CloseDocument` → `Unsubscribe`. WS opened at `Unlock`, closed at `Logout`; reconnection → automatic resubscribe.

**Embedded documents (Toile → Prose/Trame):** MVP read-only live (the Logic thread opens a viewer state, receives `RemoteUpdate`, never emits `SendUpdate`); post-MVP editable in-place (input routing by focus, dedicated spec). The nesting relationship is stored in the Toile's `LoroDoc` (encrypted content, server-invisible). No embedding Postgres table.

### C. Inter-thread Transport — `flume` channels (`ambre-app::main`)

`ambre-app/src/lib.rs` (`#[wasm_bindgen(start)]`) is the **single creation and distribution point** for channels. No other crate creates them. There is no more `worker.rs`/`bridge.rs`/`bridge_sync.rs` (Model A removed).

```rust
// ambre-app/src/lib.rs (excerpt)
const CHANNEL_CAPACITY: usize = 1024;

#[wasm_bindgen(start)]
pub fn main() {
    // tracing_wasm + console_error_panic_hook set here (panic hook -> ambreFatal, to be wired).

    // 4 bounded channels (backpressure). Worker senders may block; UI never.
    let (ui_to_logic_tx, ui_to_logic_rx) = flume::bounded::<LogicCommand>(CHANNEL_CAPACITY);
    let (logic_to_ui_tx, logic_to_ui_rx) = flume::bounded::<UiEvent>(CHANNEL_CAPACITY);
    let (logic_to_sync_tx, logic_to_sync_rx) = flume::bounded::<SyncCommand>(CHANNEL_CAPACITY);
    let (sync_to_logic_tx, sync_to_logic_rx) = flume::bounded::<LogicEvent>(CHANNEL_CAPACITY);

    wasm_thread::Builder::new().name("logic".into()).spawn(move || {
        ambre_logic::run(ui_to_logic_rx, logic_to_ui_tx, logic_to_sync_tx, sync_to_logic_rx);
    }).expect("spawn Logic thread");

    wasm_thread::Builder::new().name("sync".into()).spawn(move || {
        ambre_sync::run(logic_to_sync_rx, sync_to_logic_tx);
    }).expect("spawn Sync thread");

    // UI runs on THIS thread (the main one), driven by the closure that injects the view.
    ambre_canvas::run("ambre-canvas", move |w, h| {
        TrameView::new(ui_to_logic_tx, logic_to_ui_rx, w, h)
    });
}
```

**The Logic thread** loops on `flume::Selector::wait` (waits for the first of the two inputs `from_ui`/`from_sync`), dispatches, repeats; exits when **both** input channels are disconnected. **The Sync thread** loops on `from_logic.recv()` (blocking) until disconnection. **Dependency injection via closure**: `ambre_canvas::run<V, F>(canvas_id, make_view)` receives `F: FnOnce(f64, f64) -> V`; `ambre-canvas` never names `TrameView` — this is how it stays upstream of apps without cycles (cf. `DocView` vs `AppView`, §6).

> **Hybrid evolution (UI Chrome — see `ambre-spec-chrome.md`).** The threads/channels model above is **unchanged**; Leptos chrome is **additive on the UI thread**. The bootstrap evolves: `ambre-app` first mounts the Leptos `AppShell` (`ambre-chrome`) which owns the `<canvas id="ambre-canvas">` cell; `ambre_canvas::run` attaches to this canvas for the **content**, the `make_view` closure still injecting the active app's `DocView`. The Leptos shell and the canvas rAF loop coexist on the same thread. **Consequence:** the shell becoming owner of the canvas and deciding *which* view is mounted (tabs/multi-document) **brings forward the deadline** for the `DocView → AppView` switch (§6) — still deferred, but not for much longer. (Exact bootstrap to be frozen at POC, `ambre-spec-chrome.md §7`.)

### D. Data Modeling — Typed Data Plane (3 planes) & Interface Contracts

**No more `Vec<u8>` postcard on the UI ↔ Logic path.** Application payloads are **typed Rust values**, in a **closed world** (sums over document types) defined **in `ambre-bus`** (no new crate). Client IPC breaks down into **three planes**:

| Plane | Carried by | Payload | Logic routing |
|---|---|---|---|
| **Content** | `LogicCommand::App` / `UiEvent::{Viewport,Presence}` | `AppCommand` / `AppEvent` (typed, closed) | by `DocumentId` to `DocState` |
| **Service** | `…::Rest { id, … }` (4 directions) | `RestCommand` / `RestEvent` (**defined once**, `rest.rs`) | verbatim relay + state hooks |
| **Network** | `SyncCommand::Send*` / `LogicEvent::Remote*` + push | `Vec<u8>` (Loro network / encrypted bytes) | applied to CRDT / forwarded |

**Content plane (typed, closed).** `enum AppCommand { Trame(TrameCommand), … }` (UI → Logic); `enum AppEvent { Trame(TrameEvent), … }` (Logic → UI). Adding a document type = adding a variant (single edit point, **no trait-object, no `Box<dyn Any>`, no crate cycle**). **Composition** (embed Trame-in-Toile, sharing in a Rendez-Vous) = **recursion** on these enums (e.g. `ToileCommand::Embedded { child, command: Box<AppCommand> }`), routed by `Uuid` in the Logic thread. Future plugins = variant `Plugin(Box<dyn Plugin>)`. What stays `Vec<u8>` (real **network** bytes): Loro CRDT payloads/snapshots and Murmure channel messages, on `SyncCommand`/`LogicEvent`.

**Service plane (correlatable relay).** Almost all P3 traffic is pure relay: the UI requests an operation, Logic forwards it verbatim to Sync (REST/crypto/WS), the response flows back the same path. Rather than modeling each step as an enum variant in each of the 4 directions (the "P3 triplication": ~300 lines of 1:1 copying), the **service family is defined once** (`RestCommand`/`RestEvent` + `RequestId` in `rest.rs`) and **carried by thin envelopes** `…::Rest { id, … }`:

```text
LogicCommand::Rest { id, cmd: RestCommand }   // UI    → Logic
SyncCommand ::Rest { id, cmd: RestCommand }   // Logic → Sync
LogicEvent  ::Rest { id, evt: RestEvent }     // Sync  → Logic
UiEvent     ::Rest { id, evt: RestEvent }     // Logic → UI
```

The Logic thread's P3 dispatch reduces to **a single arm** (`Rest { id, cmd } => send_to_sync(Rest{…})`), preceded by a small `match &cmd` for the rare state-affecting ops (`Logout` clears `docs`, `DeleteDocument` removes the entry). Adding an op = one `RestCommand` variant + one `RestEvent` variant, edited in the same place. **`RequestId`** (newtype `u64`) is allocated by the **UI** via `RequestId::next()` (an `AtomicU64` shared between threads — legal thanks to shared memory), **echoed unchanged** by Logic and Sync. Single home = the `Rest` envelope.

**Naming convention** — by **destination + role** (`Command` = request to the downstream worker, `Event` = notification flowing back upwards); endpoints named by direction (`to_ui`/`from_ui`/..., no more `tx`/`rx`). Same suffix at both levels: `Command` travels in `Command`, `Event` in `Event`.

| Flow | Type | File `ambre-bus/src/` |
|---|---|---|
| UI → Logic | `LogicCommand` | `logic.rs` (+ `AppCommand`, `TrameCommand`) |
| Logic → UI | `UiEvent` | `ui.rs` (+ `AppEvent`, `TrameEvent`, `DisplayCell`) |
| Logic → Sync | `SyncCommand` | `command.rs` |
| Sync → Logic | `LogicEvent` | `event.rs` |
| (shared) | `RestCommand`/`RestEvent`/`RequestId` + business types | `rest.rs` |

**`Rest` vs explicit partition:**
- **In `RestCommand`/`RestEvent`** = **correlatable request/response** operations, **success responses only**: session, document CRUD, sharing, users, media, channels CRUD+sharing, unread, meetings.
- **Remain explicit on the envelope**: *control/content-send* (`OpenDocument`/`CloseDocument`, `OpenChannel`/`CloseChannel`, `SendChannelMessage` — fire-and-forget); *server push* (`Remote*`, `ConnectionStatus`, `SyncWarning`, `MemberRevoked`, `Channel*`, `Meeting*Participant*`); *errors* (`AuthError`/`ApiError` — response **or** spontaneous; correlation by `id` = future refinement).

#### Interface Contracts

**`ambre-core` → all crates**

```rust
pub enum DocType { Prose = 0, Trame = 1, Toile = 2, Murmure = 3 } // identical server SQL value
pub struct PublicKey(pub [u8; 32]);   // Ed25519 or X25519 depending on context
// Typed identifiers (newtypes over Uuid v7): DocumentId, UserId, ChannelId, MeetingId, SnapshotId.
pub enum Role { Owner, Editor, Viewer }
```

**Rendez-Vous is not a DocType** (ephemeral meetings, dedicated table).

**`ambre-draw`** — `DrawSurface` trait (source of truth, details in `ambre-spec-canvas.md §D`). Deps: kurbo/peniko/parley, no wgpu/krilla dep. Impls: `WgpuSurface` (`ambre-canvas::render`), `KrillaSurface` (PDF, future, Logic thread, lazy-loaded).

**`ambre-protocol` → `ambre-sync` + `ambre-server` (SOURCE OF TRUTH)** — `Update`/`Snapshot`/`Ephemeral` (with `commitment_tag` HMAC-SHA256 + `signature` Ed25519), `*PublicData`, `WsClientMsg`/`WsServerMsg` (multiplexed WS: `Subscribe`/`Unsubscribe`/`Update`/`Ephemeral`/`RequestMissing` + channel variants), `Signaling*`. Wire encoding `[version_byte: u8] [postcard(msg)]` (`encode_client_msg`/`decode_server_msg`) — **only place where postcard remains client-side**.

```rust
pub struct Update {
    pub ciphertext: Vec<u8>, pub nonce: [u8; 24], pub public_data: UpdatePublicData,
    pub commitment_tag: [u8; 32], // HMAC-SHA256(doc_key, nonce||ciphertext||public_data) -- anti key-commitment
    pub signature: [u8; 64],      // Ed25519 over (nonce||ciphertext||public_data||commitment_tag)
}
pub struct UpdatePublicData { pub document_id: Uuid, pub pub_key: [u8;32], pub snapshot_id: Uuid, pub clock: u64 }
// Snapshot/SnapshotPublicData (parent_snapshot_proof BLAKE2b, parent_snapshot_update_clocks), Ephemeral/EphemeralPublicData: same.
```

> **`commitment_tag`**: XChaCha20-Poly1305 is not key-committing; HMAC binds the message to a specific key, preventing a compromised server from re-injecting a ciphertext into another context. **Snapshots**: via S3 presigned URLs (not WS); `SnapshotAvailable` notifies. **Signaling**: the server sees SDPs (who is calling whom) — metadata leak accepted for MVP; E2EE media goes through SFrame.

**`ambre-bus` → Logic / Sync / UI threads** — the **4 envelopes** + the shared service plane. **Nothing is serialized here.** `Vec<u8>` only remain for real network bytes.

```rust
// -- UI -> Logic: logic.rs --------------------------------------------------
pub enum LogicCommand {
    App { document_id: DocumentId, command: AppCommand },          // content (P2)
    Rest { id: RequestId, cmd: RestCommand },                      // service (P3) relayed
    OpenDocument { document_id: DocumentId },                      // control (P3) fire-and-forget
    CloseDocument { document_id: DocumentId },
    OpenChannel { channel_id: ChannelId },
    CloseChannel { channel_id: ChannelId },
    SendChannelMessage { channel_id: ChannelId, payload: Vec<u8> }, // opaque payload (ambre-murmure)
}
pub enum AppCommand { Trame(TrameCommand) /* , Prose, Toile, ... ; future Plugin(Box<dyn Plugin>) */ }
pub enum TrameCommand {
    /// Monotonic `version`: UI discards any response older than the last accepted one.
    ScrollViewport { version: u64, row_start: u32, row_end: u32, col_start: u32, col_end: u32 },
}

// -- Logic -> UI: ui.rs -----------------------------------------------------
pub enum UiEvent {
    Viewport { document_id: DocumentId, event: AppEvent },         // content
    Presence { document_id: DocumentId, event: AppEvent },
    Rest { id: RequestId, evt: RestEvent },                        // service
    ConnectionStatus { document_id: DocumentId, connected: bool }, // push & status
    SyncWarning { document_id: DocumentId, code: u16, message: String },
    MemberRevoked { document_id: DocumentId, revoked_user_id: UserId },
    ChannelMessage { channel_id: ChannelId, payload: Vec<u8> },
    ChannelPresenceUpdate { channel_id: ChannelId, payload: Vec<u8> },
    ChannelConnectionStatus { channel_id: ChannelId, connected: bool },
    MeetingParticipantJoined { meeting_id: MeetingId, user_id: UserId },
    MeetingParticipantLeft { meeting_id: MeetingId, user_id: UserId },
    AuthError { message: String },
    ApiError { request: String, code: u16, message: String },
    /// Unrecoverable thread failure (GRACEFUL path). Panic, however, is
    /// panic=abort and does NOT manifest as a disconnect (cf. sec.2.B).
    FatalError { message: String },
}
pub enum AppEvent { Trame(TrameEvent) }
pub enum TrameEvent { ViewportData { version: u64, row_start: u32, col_start: u32, cells: Vec<DisplayCell> } }
pub struct DisplayCell { pub row: u32, pub col: u32, pub text: String } // flat, data-oriented

// -- Logic -> Sync: command.rs ----------------------------------------------
pub enum SyncCommand {
    Rest { id: RequestId, cmd: RestCommand },
    OpenDocument { document_id: DocumentId }, CloseDocument { document_id: DocumentId },
    SendUpdate   { document_id: DocumentId, payload: Vec<u8> },   // raw Loro CRDT delta
    SendSnapshot { document_id: DocumentId, payload: Vec<u8> },   // full state -> S3
    SendEphemeral{ document_id: DocumentId, payload: Vec<u8> },   // cursor/presence (encrypted by Sync)
    Reconnect    { document_id: DocumentId },
    OpenChannel  { channel_id: ChannelId }, CloseChannel { channel_id: ChannelId },
    SendChannelUpdate    { channel_id: ChannelId, payload: Vec<u8> },
    SendChannelEphemeral { channel_id: ChannelId, payload: Vec<u8> },
    Shutdown,
}

// -- Sync -> Logic: event.rs ------------------------------------------------
pub enum LogicEvent {
    Rest { id: RequestId, evt: RestEvent },
    RemoteUpdate   { document_id: DocumentId, payload: Vec<u8> },  // decrypted/decompressed -> LoroDoc::import
    RemoteSnapshot { document_id: DocumentId, payload: Vec<u8> },
    RemoteEphemeral{ document_id: DocumentId, payload: Vec<u8> },
    ConnectionStatus { document_id: DocumentId, connected: bool },
    SyncWarning { document_id: DocumentId, code: u16, message: String },
    SnapshotNeeded { document_id: DocumentId },                   // -> export_snapshot -> SendSnapshot
    MemberRevoked { document_id: DocumentId, revoked_user_id: UserId },
    ChannelRemoteUpdate { channel_id: ChannelId, payload: Vec<u8> },
    ChannelRemoteEphemeral { channel_id: ChannelId, payload: Vec<u8> },
    ChannelConnectionStatus { channel_id: ChannelId, connected: bool },
    ChannelSnapshotNeeded { channel_id: ChannelId },
    MeetingParticipantJoined { meeting_id: MeetingId, user_id: UserId },
    MeetingParticipantLeft { meeting_id: MeetingId, user_id: UserId },
    AuthError { message: String }, ApiError { request: String, code: u16, message: String },
}

// -- Shared service plane: rest.rs -----------------------------------------
pub struct RequestId(pub u64); // allocated by UI (RequestId::next, shared AtomicU64), echoed by Logic/Sync
pub enum RestCommand {
    // Session
    Unlock { passphrase: Vec<u8> }, SetupAccount { passphrase: Vec<u8> },
    ChangePassphrase { current_passphrase: Vec<u8>, new_passphrase: Vec<u8> },
    RecoverAccount { recovery_words: String, new_passphrase: Vec<u8> }, Logout,
    // Document CRUD
    ListDocuments, CreateDocument { doc_type: DocType, name: String },
    DeleteDocument { document_id: DocumentId }, RenameDocument { document_id: DocumentId, name: String },
    // Sharing
    InviteToDocument { document_id: DocumentId, invitee_email: String, role: Role },
    RevokeFromDocument { document_id: DocumentId, user_id: UserId },
    ListDocumentMembers { document_id: DocumentId },
    UpdateDocumentMemberRole { document_id: DocumentId, user_id: UserId, role: Role },
    // Users / Media
    SearchUsers { query: String },
    UploadMedia { document_id: DocumentId, data: Vec<u8>, mime_type: String },
    DownloadMedia { document_id: DocumentId, content_hash: String },
    // Channels (CRUD & sharing) + unread
    ListChannels, CreateChannel { name: String }, DeleteChannel { channel_id: ChannelId },
    InviteToChannel { channel_id: ChannelId, invitee_email: String, role: Role },
    RevokeFromChannel { channel_id: ChannelId, user_id: UserId },
    ListChannelMembers { channel_id: ChannelId },
    UpdateChannelMemberRole { channel_id: ChannelId, user_id: UserId, role: Role },
    UpdateReadCursor { channel_id: ChannelId, last_seq: u64 }, GetUnreadCounts,
    // Meetings
    CreateMeeting { name: String }, JoinMeeting { meeting_id: MeetingId },
    LeaveMeeting { meeting_id: MeetingId }, EndMeeting { meeting_id: MeetingId },
}
pub enum RestEvent { // SUCCESS responses only (failures = AuthError/ApiError on the envelope)
    SessionReady { user_id: UserId, display_name: String, email: String },
    AccountSetupRequired, AccountCreated { recovery_words: String },
    PassphraseChanged, AccountRecovered, LoggedOut,
    DocumentList { documents: Vec<DocumentSummary> },
    DocumentCreated { document_id: DocumentId, doc_type: DocType }, // Logic hook: pending_doc_types
    DocumentDeleted { document_id: DocumentId }, DocumentRenamed { document_id: DocumentId },
    InviteSent { document_id: DocumentId, user_id: UserId, display_name: String },
    RevocationComplete { document_id: DocumentId },
    DocumentMembers { document_id: DocumentId, members: Vec<MemberInfo> },
    MemberRoleUpdated { document_id: DocumentId, user_id: UserId, role: Role },
    UserSearchResults { users: Vec<UserInfo> },
    MediaUploaded { document_id: DocumentId, content_hash: String },
    MediaDownloaded { document_id: DocumentId, content_hash: String, data: Vec<u8> },
    ChannelList { channels: Vec<ChannelSummary> }, ChannelCreated { channel_id: ChannelId },
    ChannelDeleted { channel_id: ChannelId }, ChannelInviteSent { channel_id: ChannelId, user_id: UserId },
    ChannelRevocationComplete { channel_id: ChannelId },
    ChannelMembers { channel_id: ChannelId, members: Vec<MemberInfo> },
    ChannelMemberRoleUpdated { channel_id: ChannelId, user_id: UserId, role: Role },
    UnreadCounts { counts: Vec<UnreadCount> },
    MeetingCreated { meeting_id: MeetingId }, MeetingJoined { meeting_id: MeetingId },
    MeetingLeft { meeting_id: MeetingId }, MeetingEnded { meeting_id: MeetingId },
}
// Shared business types (rest.rs) -- zero crypto types, names already decrypted by Sync:
// DocumentSummary, MemberInfo, UserInfo, ChannelSummary, UnreadCount.
```

> **No more `AppController` or `CommandSink`** (deleted Model A traits). The document registry is **typed**: `HashMap<DocumentId, DocState>` where `enum DocState { Trame(TrameState), … }`, and dispatch is a **`match`** on `(DocState, AppCommand)` — no trait-object, no `downcast`, no `Box<dyn Any>`. Logic sends on a bare `&flume::Sender<SyncCommand>`.

**Logic thread dispatch (`ambre-logic::LogicState`)**: `handle_logic_command` routes `App` to `docs[document_id]` (drop if unknown) → `UiEvent::Viewport`; `Rest` → single arm forward `SyncCommand::Rest` (+ hooks `Logout`/`DeleteDocument`); control → counterpart `SyncCommand`. `handle_logic_event`: `Rest` → forward `UiEvent::Rest` (+ hook `DocumentCreated` → `pending_doc_types`); `RemoteSnapshot` promotes a `pending_doc_type` to a live `DocState`; `SnapshotNeeded` → `export_snapshot` → `SendSnapshot`; pushes/errors forwarded explicitly.

### E. Crates and External Dependencies

**Server (`ambre-server`, native):** `rustis`, `sqlx`, `aws-sdk-s3`, `axum`/`tower`/`hyper`, `postcard`/`serde`/`serde_json`, `thiserror`/`tracing`, `ed25519-dalek` (direct).

**WASM client (single binary, shared dependencies):**
- `wasm_thread` — WASM thread spawning (Web Workers loading the same module + shared memory).
- `flume` (`select` feature) — MPMC inter-thread channels (Logic/Sync rely on `Selector`).
- `wgpu` (+ `webgl`) + `swash` — GPU rendering of **content** (UI thread). WebGL2 by default, WebGPU opt-in `?webgpu`.
- `parley` — typographic layout (UI + Logic pagination). Fonts via same-origin `fetch()`.
- `leptos` — **chrome DOM** (UI thread; shared `ambre-chrome` + app-specific chrome). `nucleo-matcher` — fuzzy palette.
- `accesskit` — a11y of canvas **content** (scoped; chrome DOM is natively accessible). `kurbo`/`peniko` — types (`ambre-draw` deps).
- `loro` — CRDT (Logic). `petgraph` — formula DAG (Trame). `rstar` — R-Tree (Toile).
- `web-sys`/`js-sys`/`wasm-bindgen`/`wasm-bindgen-futures` — browser glue (UI `host` + Sync).
- `gloo-net` (websocket+http) — network (Sync). `ruzstd` — compression. `zeroize` — secrets.
- `tracing` + `tracing-wasm` + `console_error_panic_hook` — logging + panic hook (set in `ambre-app`).
- Crypto (future, `ambre-crypto`): `chacha20poly1305`/`ed25519-dalek`/`x25519-dalek`/`blake2`/`argon2`.
- `krilla`/`docx-rs`/`html5ever`/`pulldown-cmark` — import/export interop (lazy-loaded, Logic).
- `postcard` — **only** the network wire (`ambre-protocol`).

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

**Blind Relay boundaries:**

| Component | Data seen by server | Never seen |
|-----------|----------------------------|-------------|
| `ambre-server` | `document_id`, `pub_key` (Ed25519), `clock`, `seq`, `snapshot_id`, `nonce` | CRDT content, Document Key, Master Seed, User Key, passphrase |

**Static verification via `cargo tree`:** `ambre-server` imports no type from `ambre-trame`/`ambre-bus`/`ambre-crypto`; it imports `ed25519-dalek` directly (not `ambre-crypto`). All `ciphertext` in `ambre-protocol` are opaque `Vec<u8>`.

**Key separation:**
- `DocumentKey` and `MasterSeed` (`ambre-crypto`) implement `ZeroizeOnDrop`. Never serialized in `ambre-protocol` or `ambre-bus`.
- The `flume` Logic ↔ Sync channels carry **decrypted** CRDT deltas (`Vec<u8>`, by move in the shared heap). Internal boundary considered safe: the heap is only accessible from within the user's browser process. **Shared** memory removes Model A's "binary isolation" argument — trust rests on the process sandbox and the crate graph (the UI thread doesn't compile crypto, etc.).
- Ephemeral presence data **encrypted** before sending (`Ephemeral` payload).

> **Secret transit through the Logic thread (minor, documented).** The passphrase travels `UI → Logic → Sync`. Logic **moves** it without copy or use, but these bytes reside for an instant in the **shared** heap and `zeroize` is not yet wired. **No code fix**: zeroizing on the `ambre-bus` side would impose a `zeroize` dep, **violating the `ambre-bus → ambre-core` only invariant**. Zeroization remains `ambre-sync`'s responsibility (§7 ARCH P1). Assumed choice.

**Cryptography & key management.** All crypto operations execute in the **Sync thread** (sole consumer of `ambre-crypto`). Primitives: XChaCha20-Poly1305 (AEAD), Ed25519 (signature), X25519 (ECDH), BLAKE2b (derivation/hash), Argon2id (KDF passphrase), HMAC-SHA256 (`commitment_tag`).

**E2E key hierarchy (client-side):** Postgres fields (`master_key_enc`, `master_key_salt`, `doc_key_enc`, `org_master_key_enc`) are opaque blobs to the Blind Relay (see `ambre-spec-server.md §2.A`).

```
Passphrase (UI input → flume channel transit → Sync thread, NEVER transmitted to server)
  │  Argon2id(salt=users.master_key_salt)  [Sync thread]
  ▼
User Key (ephemeral, derived at each login)
  │  XChaCha20-Poly1305 decrypts users.master_key_enc
  ▼
Master Seed (256 bits, permanent, stored encrypted server-side)
  ├── BLAKE2b(b"ambre-signing-key-v1", seed)  → Signing Key Ed25519  → signing_pubkey (Postgres)
  └── BLAKE2b(b"ambre-exchange-key-v1", seed) → Exchange Key X25519   → exchange_pubkey (Postgres)
                                                   │  X25519 ECDH with owner's exchange_pubkey
                                                   │  → decrypts document_members.doc_key_enc
                                                   ▼
                                               Document Key (one per doc, 256 bits)
                                                   │  XChaCha20-Poly1305
                                                   ▼
                                               CRDT Content (snapshots + updates + ephemeral)
```

**TOFU:** on first invitation, the UI displays the BLAKE2b-256 fingerprint (base32) of the `exchange_pubkey`. Accepted keys stored in `IndexedDB` (`trusted_keys`). Any subsequent divergence alerts. Limitation: substitution possible on first exchange if not verified out-of-band.

**Passphrase change:** new passphrase → Argon2id → new User Key → re-encrypts `master_key_enc` only (Master Seed unchanged) → `PUT /api/keys/master/rotate`. **Sharing / post-revocation rotation**: `ambre-spec-sync.md §3` (15 steps). **Multi-device, Recovery Key (24-word BIP39), Org Recovery Key (post-MVP — Shamir/Vault escrow via `OrgKeyStore` trait)**: unchanged by the move to Model B; tables (`organizations`, `org_recovery_log`) and routes (`POST /api/organizations/{id}/recover/{uid}`) in `ambre-spec-server.md §2`.

```rust
// ambre-crypto/src/key_store.rs (target)
pub struct MasterSeed(pub [u8; 32]); // ZeroizeOnDrop, 24 BIP39 words (Recovery Key)
impl MasterSeed {
    pub fn derive_signing_key(&self) -> ed25519_dalek::SigningKey { /* separate BLAKE2b ctx */ }
    pub fn derive_exchange_key(&self) -> x25519_dalek::StaticSecret { /* separate BLAKE2b ctx */ }
}
```

---

## 4. QUALITY & TESTABILITY

* **Unit tests / host testability.** As the Logic thread is **100% pure Rust** (zero WASM glue), all command/event routing is exercised **on the host** (x86_64) with mock `flume` channels: WASM rustflags are scoped `[target.wasm32-unknown-unknown]`, so `cargo test -p ambre-logic` runs natively. **This is a first-order benefit of the decomposition** (not just a dep graph property). `ambre-logic` has **9 tests** (`#[cfg(test)]`) building `LogicState` with two drainable receivers and verifying: P3 forward → `SyncCommand` with **`id` echo**; field preservation; `App` routed / drop on unknown doc; `DocumentCreated` → `RemoteSnapshot` → live state promotion; `SnapshotNeeded` → `SendSnapshot`; `Logout` clears `docs`; `CloseDocument` drop + forward. Extend to `ambre-sync` when its pipeline exits placeholder.

* **Per-crate strategy:**

| Crate | Unit tests | Integration tests | Prop-tests | Benchmarks |
|-------|----------------|---------------------|------------|------------|
| `ambre-core` | 100% of types | — | — | — |
| `ambre-crypto` | Round-trip, tampering, zeroize | — | ✓ (proptest) | ✓ (criterion) |
| `ambre-protocol` | Serialization round-trip | — | ✓ (postcard roundtrip) | — |
| `ambre-bus` | Envelope construction | — | — | — |
| `ambre-logic` | **Typed dispatch on host (9 tests, flume mock)** | — | — | — |
| `ambre-sync` | Crypto pipeline, quarantine, rotation | `MockBlindRelay` in-process | ✓ (clock monotonic) | ✓ (encrypt/decrypt 1KB) |
| `ambre-canvas` | TextCache, WgpuSurface (MockSurface), dirty/RepaintPlan | headless | — | frame budget |
| `ambre-commands` | Registry lookup, keybindings, derived state (`CommandState`) | — | — | — |
| `ambre-chrome` | Leptos components (ARIA a11y patterns), menu positioning (flip) | `wasm-bindgen-test` (DOM) | — | — |
| `ambre-server` | Routes, ACL, Lua Redis | Docker Compose | ✓ (seq ordering) | ✓ (latency p99) |
| `ambre-trame` | `TrameState` handlers, formulas, cycle detection | via `ambre-logic` | ✓ (determinism) | ✓ (recalc 10k) |
| `ambre-app` | — | End-to-end headless | — | — |

* **Mocks & environment:** native tests in `#[cfg(not(target_arch = "wasm32"))]`; WASM tests (GPU/DOM) via `wasm-bindgen-test`.

---

## 5. PERFORMANCE & RESOURCES

* **Per-operation budget (target: mobile hardware 2024):**

| Operation | Target (P95 mobile) | Ceiling | Thread | Method |
|-----------|-------------------|---------|--------|---------|
| `flume` send/receive (typed envelope move, shared heap) | < 1 µs | — | all | benchmark |
| Scroll round-trip UI → Logic → UI (P1) | < 2 ms | — | UI ↔ Logic | benchmark e2e |
| Outbound encryption 1KB (compress+AEAD+HMAC+Ed25519) | < 200 µs | < 500 µs | Sync | criterion |
| Inbound decryption 1KB (verify+HMAC+AEAD+decompress) | < 350 µs | < 1 ms | Sync | criterion |
| Full crypto round-trip (excluding network) | < 600 µs | < 1.5 ms | Sync | criterion |
| Snapshot compression 100KB (ruzstd) | < 5 ms | — | Sync | criterion |
| `doc.import()` Loro update (1KB delta) | < 2 ms | — | Logic | criterion |
| Trame formula DAG recompute (10k cells) | < 50 ms | — | Logic | custom benchmark |
| Scroll viewport response (P1, end-to-end) | < 15 ms | — | Logic → UI | benchmark e2e |

> Shared-memory `flume` **eliminates** Model A's boundary costs (postcard serialization + 2 WASM↔JS memcpy per hop): a `Vec<u8>` changes thread **by move**. Dominant remaining cost: woke-thread scheduling. Crypto bottleneck: Ed25519 (~80–250 µs). IPC smoke-test: RTT ~1–17 ms (bounded by the rAF boundary), `flume` overhead < 1 ms.

* **WASM Impact.** Model B = **single binary**: dependencies deduplicated once (no more triple copy of RustCrypto/loro between binaries as Model A feared). Total budget < 2 MB gzip; "without Prose interop" (~429 KB gzip) fits; Prose interop (`docx-rs`/`krilla`/`html5ever`/`pulldown-cmark`) stays **lazy-loaded** (bundle splitting in the Logic thread, not an extra thread). Heap: **single shared** (one `WebAssembly.Memory`, max 4 GiB); `SyncState` < 1 MB, `SyncDocumentState`/doc < 1 MB, `LoroDoc` < 10 MB target, quarantine capped 1000 updates/doc, Trame DAG < 50 MB for 100k cells.

---

## 6. REJECTED ARCHITECTURAL DECISIONS

> ⚠️ **Reversal from v1 of this spec.** Model A (shared-nothing, `postMessage`, 3 binaries) **was** the chosen architecture and listed `SharedArrayBuffer`/`flume`/`wasm_thread` as rejected. The code has switched to **Model B**: these three technologies are now **adopted** (see §1/§2). The table below documents **what remains rejected** — Model A included — so it is not re-proposed.

| Alternative | Rejected because |
|-------------|-------------|
| **Model A — shared-nothing (3 WASM binaries, `postMessage` + `ArrayBuffer` transfer + `postcard`)** | Imposed `postcard` serialization + **two WASM↔JS memcpy per hop**, and **forbade `flume`** (an `Arc` from one Worker points to memory absent in the other). Model B (shared memory) moves **typed Rust values by move**, without serialization or copy, including application payloads. Accepted cost of Model B: HTTP COOP/COEP headers (`SharedArrayBuffer`) + nightly toolchain `-Z build-std`. Do not go back to shared-nothing: we would lose the typed data plane and transport performance. |
| **Separate JS Web Workers + `postMessage` (Model A) / `MessageChannel`/`MessagePort`** | Replaced by `wasm_thread::spawn` (Web Workers loading the **same** module + importing the **same** shared memory) and `flume` channels. No more `worker.rs`/`bridge*.rs`, no more `web-sys` `MessageChannel`/`MessagePort`/`Worker*` features. |
| **`AppController` / `CommandSink` (trait-objects) + registry `HashMap<Uuid, Box<dyn AppController>>`** | Replaced by the **typed data plane**: `enum DocState` + `HashMap<Uuid, DocState>` + `match` dispatch. No trait-object, no `downcast`. A trait-object would reintroduce a potential crate cycle (the closed enum resolves downstream, in `ambre-logic`) and would prevent simple host testability. |
| **`Box<dyn Any>` / `Vec<u8>` postcard for application payloads** | Replaced by `AppCommand`/`AppEvent` (closed typed sums). The compiler guarantees dispatch exhaustiveness; adding a doc type = one variant. `Vec<u8>` only remain for **real network bytes** (Loro deltas/snapshots, channel messages). |
| **Per-direction P3 enums (4 nearly identical enums, "triplication")** | ~300 lines of 1:1 copying prone to "wrong field copied" bugs. Replaced by the `RestCommand`/`RestEvent` family **defined once** and carried by the `…::Rest { id, … }` envelope; P3 dispatch = a single arm. |
| **New crate for the typed data plane** | Unnecessary: `AppCommand`/`AppEvent` live in `ambre-bus` (upstream leaf). A dedicated crate would create a cycle or superfluous dependency. |
| **Injected effect-sink for edit outcome (towards Sync)** | Would reintroduce a trait-object (the inverse of removing `CommandSink`). Choice: **return typed outcome** (`TrameOutcome { display, update, ephemeral }`, domain bytes) decomposed by the Logic dispatcher — preserves purity + host testability. |
| **Makepad** | `script_mod!` macro bypassing the Rust type system, incompatible with the security pillar (no homegrown `unsafe`). |
| **Single crate (`ambre-all`)** | Excessive compile times, violation of server/client separation. |
| **JSON for the network wire** | `postcard` ~10x more compact, faster parsing. (On the inter-thread path, **nothing is serialized**: Rust values on `flume`.) |
| **`ambre-crypto` in `ambre-server`** | Least privilege: would expose `DocumentKey`/`MasterSeed` to the server. Direct `ed25519-dalek` suffices. |
| **Wire types redefined in `ambre-sync`/`ambre-server`** | Guaranteed divergence (`[u8; 32]` vs `Vec<u8>`). Single source of truth = `ambre-protocol`. |
| **`ambre-sync`/`ambre-logic` as direct dependency of apps** | Strong logic↔infra coupling. Apps only know `ambre-bus` IPC types. |
| **Logic + Sync merged (single thread)** | Decryption/decompression would block response to P1 events (scroll). Separation mandatory (< 15 ms). |
| **`ambre-canvas` depending on apps (`AppView` closed enum today)** | Would create a cycle (`ambre-canvas` is upstream of apps). Hence `DocView` + closure injection. Switching to `AppView` remains possible **but requires moving `FrameHost` out of `ambre-canvas`** — deferred to multi-document (§7 ARCH P2). |
| **100% canvas UI (homemade widget framework) + Vello/Masonry/Xilem** | Rejected in favor of **hybrid UI** (Leptos DOM chrome + wgpu content, web-only). Details and related rejections: UI model (Masonry, Xilem, winit, WebGPU-only) in `ambre-spec-ui.md §6`, chrome (egui, ribbon) in `ambre-spec-chrome.md §6`, content (Vello) in `ambre-spec-canvas.md §6`. The `ambre-canvas` GPU/text pipeline is **preserved**; only the widget framework was removed. |
| **One WebSocket per document** | Wastage (browser limits ~6 conns/origin, battery). Single multiplexed WS, Subscribe/Unsubscribe per doc. |
| **Separation `ambre-sync` / `ambre-network`** | The network layer is lightweight enough to stay in `ambre-sync`. Separating would create a cycle or complexity. |
| **Complex DI framework** | The document registry is a `HashMap`; injection (closure `make_view`, channels) is explicit in `ambre-app`. No DI framework. |
| **4th thread for import/export** | Unnecessary: during an import/export the UI shows a waiting screen; the Logic thread blocked (on a frozen `LoroDoc` version) is the intended behavior. WASM lazy loading, not an extra thread. |

---

## 7. INTEGRATION PLAN (ROADMAP)

> Detailed backlog of this spec; cross-cutting scheduling (milestones) lives in `specs/ambre-roadmap.md`.

### ARCH P0 — Foundation
- [x] Workspace + `rust-toolchain.toml` (nightly, `rust-src`, `wasm32`) + `.cargo/config.toml` (`+atomics`, `--shared-memory`, `-Z build-std`).
- [x] Validated shared-memory threading prototype: 3 `wasm_thread` threads + 4 `flume` channels + shared heap. IPC smoke-test operational.
- [x] `ambre-core`; `ambre-protocol` (encode/decode, round-trip); `ambre-bus` (4 envelopes + service plane, zero serde); `ambre-draw`.
- [x] `ambre-crypto` (RustCrypto wrappers, `ZeroizeOnDrop`, prop-tests, benchmarks; `seal`/`open` envelope + primitives).

### ARCH P1 — Threading & Sync
- [x] Decoupled runtimes (`ambre_canvas`/`ambre_logic`/`ambre_sync::run`), `ambre-app` launcher.
- [x] Backpressure (bounded channels), non-blocking UI sends + coalescing, `FatalError`, typed dispatch tested on host (9 tests).
- [x] `ambre-crypto` pipeline (+ `commitment_tag`) + causal quarantine by `snapshot_id` (`clock` ASC release, cap 900/1000).
- [x] `RestCommand::Unlock` + `SetupAccount`; `OpenDocument`/`CloseDocument` (multi-document).
- [x] Real multiplexed WebSocket.
- [x] Non-blocking Sync loop (`recv_async` + WS via `JsFuture`/`spawn_local`): blocking `recv()` freezes the Worker's event loop (`Atomics.wait`).
- [x] Anti-deadlock Logic <-> Sync: Logic -> Sync blocking (lossless), Sync -> Logic via `try_send` (on `Full`: stop reading WS -> TCP backpressure + `RequestMissing`).
- [ ] Separate UI -> Logic traffic classes: lossy viewport (drop + coalescing) vs lossless edit/service (synchronous failure at call-site on `Full`).
- [ ] Consume `RequestId` on the UI side: `RequestRegistry<Tag>` generic in `ambre-canvas` (routes `UiEvent::Rest`, timeout/retry; `RequestId::next()` sole allocator).
- [x] `ambreFatal(message)` hook in `panic::set_hook` (before abort): UI surface for non-graceful panic.
- [x] `SnapshotRequired` -> `SnapshotNeeded` -> `SendSnapshot`. `MockBlindRelay`.
- [x] Remove smoke-test scaffolding (`IPC_TEST_DOC_ID`, pre-seeded `DocState::Trame`, `pending_doc_types`) once the real open-document flow arrives.

### ARCH P2 — Pilot Applications
- [x] `ambre-trame`: `TrameState` + `TrameView` (impl `DocView`), virtual-scroll grid, viewport smoke-test.
- [x] Hybrid UI migration: `ambre-commands` + `ambre-chrome` created and mounted by `ambre-app`; homemade widget framework removed, `ambre-canvas` slimmed to render runtime.
- [x] Trame editing: `TrameState::handle_command` returns `TrameOutcome { display, update, ephemeral }`, decomposed by the Logic dispatcher (lossless path; subsumes snapshot export off the hot path).
- [ ] Extract Trame-specific chrome from `ambre-app` into `ambre-trame`'s `chrome` module.
- [ ] `DocView` -> `AppView` switch (move `FrameHost` out of `ambre-canvas`), at multi-document/tabs.
- [ ] `ambre-prose` (LoroText).

### ARCH P3 — Blind Relay
- [ ] `ambre-server`: REST routes, WS upgrade, ACL, Lua Redis. PostgreSQL/Valkey/MinIO. Snapshot via S3. E2E tests.

### ARCH P4 — Finalization & Optimization
- [ ] Full Trame formula engine (DAG, incremental recompute, cycle detection).
- [ ] Local Loro persistence (IndexedDB) + offline durability: persist Loro state, compact offline outbound queue, flush + `RequestMissing` on reconnect; passive badge fed by `ConnectionStatus`.
- [ ] Document Key rotation (15 steps), full CRUD/sharing/media. Hot path benchmarks. ZK audit (`cargo tree`).
- [ ] Audit wasm-only warnings (`-A warnings` masks dead code on `ambre-app`/`ambre-canvas::host`/`ambre-sync`); confirm `CanvasRenderingContext2d`/`ImageData` are dead.

### ARCH P5 — Remaining Applications
- [ ] `ambre-toile` (R-Tree + read-only embeds), `ambre-murmure` (channels, segments managed by Sync).
- [ ] Composition (`AppCommand`/`TrameOutcome` recursion routed by `Uuid`). Encrypted presence. TOFU UI. Meetings (WebRTC signaling).
