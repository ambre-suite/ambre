# AMBRE - E2EE collaborative office suite, 100% Rust.

**End-to-End Encrypted, 100% Rust**

AMBRE is a next-generation collaborative office suite built entirely in Rust, featuring real-time
collaboration with CRDTs, end-to-end encryption (E2EE), and GPU-accelerated rendering via wgpu
(WebGL2/WebGPU). It runs as a single-page WebAssembly application with a Rust-native server backend.

## Vision & Identity

AMBRE is a collaborative office suite that is:

- **100% Rust** -- both client (WASM) and server
- **End-to-End Encrypted (Zero-Knowledge)** -- the server acts as a blind relay
- **Open Source** -- fully transparent development
- **European cloud** -- hosted in the EU

**Differentiator.** The stack is not the differentiator. A user never sees it, and a coding agent
reproduces it at a low cost. Ambre differs by the guarantees it states and the evidence it
publishes for each one:

| Guarantee | Where it is stated |
|-----------|--------------------|
| The relay stores every document and can read none of it: no content, no key. | [`ambre-spec-zero-knowledge.md`](specs/1-Foundations/ambre-spec-zero-knowledge.md) |
| A member removed from a document cannot read what is written after the removal. | [`ambre-spec-key-rotation.md`](specs/1-Foundations/ambre-spec-key-rotation.md) |
| A change reaches another peer with the changes it was built on, or it waits. No peer shows half a document. | [`ambre-spec-delta-order.md`](specs/1-Foundations/ambre-spec-delta-order.md) |
| An office file opens in Ambre and leaves it again, and a report names each construct lost. | [`ambre-spec-interop.md`](specs/1-Foundations/ambre-spec-interop.md) |

Each guarantee is a set of contract rows, and each row names the mechanism that verifies it. The
[contract ledger](specs/1-Foundations/ambre-contract-ledger.md) publishes the status of every row,
including the rows that no mechanism holds yet. A clone of the features carries none of this
evidence.

## Applications

| Application | Description |
|-------------|-------------|
| **Prose** | Collaborative word processing |
| **Trame** | Collaborative spreadsheet: calculation engine, formulas, dependency DAG |
| **Toile** | Infinite whiteboard and presentations. **A Miro, not a Figma** |
| **Murmure** | Secure messaging with E2EE channels, and E2EE audio and video calls started from a channel |

## Core Principles (Non-Negotiable Pillars)

| Pillar | Constraint |
|--------|------------|
| **Rust-first** | 100% Rust client and server. Pure Rust libraries preferred over C bindings. |
| **Pure CSR** | Single Page Application in WASM. All computation client-side; server only relays. |
| **E2EE** | Server is a blind relay: it never sees plaintext content. |
| **Security** | No `unsafe` in our code. TOFU (Trust On First Use) model with out-of-band verification. A key transparency log is planned for V2. |
| **Performance** | Mobile-first targets (see below). |
| **Zero-Knowledge** | Server only knows routing metadata (document_id, public key, clock, sequence). No content, no encryption keys. |

## Performance Targets (Mobile-First)

All targets calibrated for a mid-range 2024 smartphone. The per-operation budgets derived from them
are normative in [`specs/1-Foundations/ambre-spec-budgets.md`](specs/1-Foundations/ambre-spec-budgets.md),
which is the one register every budget of the workspace is a row of.

| Requirement | Target | Justification |
|-------------|--------|---------------|
| **Render framerate** | 60+ FPS | Smooth interaction, no jank. Render thread must never be blocked by computation. |
| **Content display latency** | < 15 ms | Between user action (scroll, input) and visual update. Users should perceive no lag. |
| **Inter-user latency** | < 300 ms | Between one user's input and display on a collaborator's screen. Network-dependent, but client processing adds no more than 50 ms. |
| **Initial load time** | < 3 s on 4G | First interactive display. First wasm download < 4 MB brotli, one wasm for the whole suite. |
| **Client memory** | < 100 MB | For a typical document. Proportional to document size, not its history. |

**Architectural orientation:** 3 isolated WASM workers (render / business logic / encrypted sync),
GPU rendering via wgpu, real-time CRDT collaboration.

## Technology Choices

| Domain | Choice | Justification |
|--------|--------|---------------|
| **Language** | Rust -> WebAssembly (client), native Rust (server) | Memory safety without GC, performance, compiler as safety net for generated code. |
| **Rendering** | wgpu + lyon + Parley/Swash | Vertex/fragment shaders only. Path tessellation via lyon, text via Parley/Swash glyph atlas. No DOM HTML for application UI. WebGPU is the target backend, WebGL2 the current default (`ambre-canvas`, see below). |
| **Collaboration** | CRDT | Conflict resolution without a central truth server. Compatible with E2EE (no server content understanding). Offline-first editing. |
| **Internal serialization** | Compact binary format | IPC between workers and network protocol. Small, fast to parse, no textual overhead. JSON only for REST API. |
| **Real-time transport** | Binary WebSocket | CRDT deltas, presence, cursors. Persistent connection, low per-message overhead. |
| **Calls** | Browser WebRTC, each chunk sealed in Rust, an SFU in Rust over str0m | The browser alone holds the capture, the codecs and the echo canceller. The SFU forwards chunks it cannot open (`specs/1-Foundations/ambre-spec-call-media.md`). |
| **HTTP transport** | HTTP/2 minimum, HTTP/3 (QUIC) targeted | Multiplexing (HTTP/2), reduced mobile latency (QUIC eliminates TCP head-of-line blocking). |
| **Encryption** | Modern pure-Rust primitives | AEAD (XChaCha20-Poly1305), signatures (Ed25519), key exchange (X25519), hash (BLAKE2b), KDF (Argon2). No OpenSSL bindings. |
| **Compression** | zstd, pure Rust implementation | Applied before encryption. WASM-compatible. |

WebGL2 is the default backend because a compositor bug on some drivers cannot be detected from
JavaScript. WebGPU is opt-in through the `?webgpu` URL flag.

## Rejected Alternatives

Product and stack alternatives, evaluated and rejected. Rejected *architectural* decisions, such as
channel topology, worker model and dispatch, are listed in
[`specs/1-Foundations/ambre-spec-architecture.md`](specs/1-Foundations/ambre-spec-architecture.md),
section 4.

| Alternative | Rejected because |
|-------------|-------------------|
| **JavaScript / TypeScript** | Incompatible with the Rust-first pillar. No memory safety guarantee. Performance insufficient for heavy client-side computation (formulas, CRDT, crypto). |
| **DOM / HTML for the application UI** | Performance insufficient for 60 FPS rendering of complex documents (a 100k-cell spreadsheet, an infinite whiteboard). The DOM is not designed for high-performance custom rendering. |
| **Raw WebGL, without the wgpu abstraction** | Low-level API with no 2D abstraction and no portability across backends. wgpu drives WebGL2 and WebGPU from one code path, which is what makes the default above switchable. |
| **OnlyOffice** | No native E2EE. Server-centric architecture, incompatible with the blind relay model. |
| **CryptPad** | JavaScript stack, OT instead of CRDT, DOM rendering. Fails the Rust-first and Performance pillars. |
| **Electron / Tauri** | AMBRE is a web application (browser PWA). No native desktop client: one codebase, one deployment. |
| **Matrix / XMPP for Murmure** | Murmure rides the Ambre sync pipeline, so one model, one key hierarchy and one relay serve every application of the suite. |

## Project Structure

AMBRE is organized as a Cargo workspace with the following crates:

| Crate | Purpose |
|-------|---------|
| `ambre-core` | Core types, UUID, versioning, errors |
| `ambre-crypto` | Cryptographic primitives (AEAD, signatures, KDF) |
| `ambre-protocol` | Wire format, WebSocket messages, signaling |
| `ambre-bus` | Inter-worker communication |
| `ambre-crdt` | The Loro map handles typed by the keys of one schema, shared by the four applications |
| `ambre-draw` | `DrawSurface` trait + shared primitives (kurbo, peniko, parley types) |
| `ambre-canvas` | Content rendering runtime for the UI thread (wgpu) |
| `ambre-pdf` | `DrawSurface` implementation backed by krilla, for PDF export |
| `ambre-convert` | The converter modules: one wasm instance per format family, which the Logic thread fetches on the first conversion of that family |
| `ambre-opc` | The package layer of the office formats: the zip archive, the content types, the relationships and the bounds of a hostile package, shared by the three codecs below |
| `ambre-pptx` | The PowerPoint codec: a deck model, and a writer of pure functions over the package layer and quick-xml |
| `ambre-xlsx` | The xlsx codec: an Excel workbook read into a plain model and written back, pure functions over the bytes |
| `ambre-docx` | The Word codec: a `.docx` package read into a model of the format and written back, comments and tracked changes included |
| `ambre-chrome` | Shared, app-agnostic Leptos chrome toolkit |
| `ambre-commands` | Shared command registry for the UI thread |
| `ambre-logic` | Logic thread entry point: command and event routing, pure Rust |
| `ambre-sync` | CRDT synchronization layer |
| `ambre-server` | Blind-relay server (Axum, PostgreSQL, Redis) |
| `ambre-sfu` | Selective forwarding unit of a call: forwards sealed chunks it cannot open (str0m, RustCrypto) |
| `ambre-trame` | Spreadsheet application |
| `ambre-prose` | Word processor application |
| `ambre-toile` | Whiteboard/presentation application |
| `ambre-murmure` | Secure messaging |
| `ambre-app` | Main WASM application (UI integration). High-level integration and the shell only: the logic of an application lives in its own crate (`ARCH-018`) |
| `ambre-belt` | Test-only: the browser tier, which drives the served build in headless Chrome. No product crate depends on it |
| `ambre-primitives` | Host-only gate: every bare primitive outside a test is typed or named by the allowlist (`ARCH-033`), and the crate boundaries hold, read through rustc and `cargo metadata`. No product crate depends on it |

## Specifications

The `specs/` directory is the source of truth for behaviour. Layout:

| Path | Content |
|------|---------|
| [`specs/ambre-roadmap.md`](specs/ambre-roadmap.md) | The only place anything is scheduled: every epic and every story, one row each. No spec carries a backlog. |
| [`specs/ambre-roadmap-journal.md`](specs/ambre-roadmap-journal.md) | Closed epics, cut here verbatim the day they closed. What shipped, and under which epic. |
| [`specs/ambre-spec-template.md`](specs/ambre-spec-template.md) | The contract every spec follows: the header fields, the contract table, the rule that keeps implementation out, and the section order of the two document kinds. |
| `specs/1-Foundations/` | Horizontal specs: the three-thread stack, the message bus, the zero-knowledge boundary, the persisted schema and testing, then one document per shared subsystem -- client side, the Sync worker, the sealed envelope, delta order, membership, key rotation, the media transfer; relay side, the blind relay, the session, the sequencer, the broadcast, the blob store, organizations -- and one per shared feature: the UI thread, the frame, the render assets, the drawing surface, the inline text editor, high density, the chrome shell, the text input pipe, accessibility, the adaptive layout, the launcher, key verification, the command model, the design tokens, the clipboard, presence, the native file, the fixtures. It also holds four things that are not specs: the budget register, the generated inventory of the layer, the generated contract ledger and the generated pair the ledger cannot answer for -- one record per contract row, and one edge per citation those rows make. |
| `specs/2-Trame/`, `specs/3-Toile/`, `specs/4-Prose/`, `specs/5-Murmure/` | Vertical specs, one directory per application. |
| `specs/<app>/features/` | One document per feature, always current: its contract, its model and its whole surface. A feature is the smallest thing a user can name, ship alone and specify end to end. |
| `specs/Todo/` | Sketches of an application that has not started. None today. |
| `research/` | Working notes that are not specs: the evidence a design decision rests on, such as the Murmure user research. |
| [`specs/audits/security-audit.md`](specs/audits/security-audit.md) | Numbered audit findings. A contract row that closes a finding cites it. |

An application directory holds a schema document, a generated inventory and one document per
feature. One of those feature documents is the **engine**. It takes what no single feature
owns: the CRDT state, the command and event planes, and the convergence rules of the document tree. The
three started applications all follow this shape.

Every spec declares its rules as a contract table, one falsifiable rule per row. Three documents
govern them, and each answers one question:

* **What a rule looks like** -- the grammar, the four status values, the binding of an unbuilt row
  to a story: [`specs/ambre-spec-template.md`](specs/ambre-spec-template.md). Normative. The four
  are `verified`, `failing`, `unimplemented` and `human-review`.
* **How a rule is verified** -- the mechanism ladder, the four tiers, the regime per component
  shape: [`specs/1-Foundations/ambre-spec-testing.md`](specs/1-Foundations/ambre-spec-testing.md).
  Normative.
* **What every rule's status is today** --
  [`specs/1-Foundations/ambre-contract-ledger.md`](specs/1-Foundations/ambre-contract-ledger.md),
  generated by `just contracts-write` and never edited by hand.

A tool reading a rule reads neither the corpus nor that ledger. `ambre-contract-index.json` carries
every row as written, sources parsed; `ambre-contract-graph.json` carries the edges those rows
assert. Neither is committed: `just index-write` builds both into `target/specs/`, which git
ignores, and `just index` holds their edges. Neither is authoritative: the Markdown is.

`just spec-context <ID>` compiles what building one rule takes. The row, the frame conditions beside
it, its operation, the design sections it cites, the rules it depends on, its mechanism, its gap.
The order is `ambre-spec-template.md section 1.H`. At the median it is a third of a full read.

`just spec-context-story "<NAME>"` compiles the same closure for every row one story lands, because
work starts at a story and the closure starts at an id. A block two rows share is quoted once and
named after. A story landing no row says so: the rules it writes do not exist yet.

The CI `lint` job runs `just contracts`, `just index` and `just spec-code` over the corpus.

`just spec-read <file>` prints one document's rules alone -- the id, the kind and the statement,
without the four columns a review reads. It writes nothing, and the file stays the only source.

## Getting Started

### Prerequisites

- `rustup`. The first `cargo` command installs the pinned nightly of `rust-toolchain.toml`, with its
  `wasm32-unknown-unknown` target and `rust-src`. No other toolchain builds Ambre (`ARCH-075`).
- `trunk` for building and serving the WASM frontend
- PostgreSQL and Redis 7.4 or later (for the server)

### Building

```bash
# Build all crates
cargo build --release

# Build the WASM application
cd crates/ambre-app
trunk build --release
```

### Running the Server

```bash
./run_server.sh
```

The script loads `.env` and runs `cargo run -p ambre-server`. Bring the backing services up first
with `docker compose up -d`. The server starts on `http://localhost:3000`. The frontend is served
by `trunk serve` (default port `8080`).

### Running the Tests

```bash
./run_tests.sh            # lint + native + db + wasm
./run_tests.sh db         # one tier: all | lint | native | db | wasm
```

The `justfile` holds the exact command lines CI runs, one recipe per tier. It does nothing else:
the db tier fails when called through `just` alone, because the recipe neither starts the backing
services nor loads `.env`. `run_tests.sh` wraps the same recipes and adds those two steps, then
tears the services down on exit unless `KEEP_SERVICES=1`. Use the script locally and `just` to
reproduce a CI job bit-for-bit.

## Roadmap

Open work is in [`specs/ambre-roadmap.md`](specs/ambre-roadmap.md), one row per story. What already
shipped is in [`specs/ambre-roadmap-journal.md`](specs/ambre-roadmap-journal.md).

## License

AMBRE is open-source software licensed under the
[GNU Affero General Public License v3.0 (AGPL-3.0)](LICENSE).

This license ensures that:
- The software remains free and open-source
- Any modifications to the software must be shared with the community
- Network services using AMBRE must make their source code available to users

See the `LICENSE` file for full terms and conditions.

## Contributing

Only a repository collaborator may open a pull request. Issues stay open to everyone, and a
reproduced bug is welcome. [`CONTRIBUTING.md`](CONTRIBUTING.md) states what each channel accepts,
the checks no gate performs, and the contract rows a reviewer holds by hand. Agents work from
[`AGENTS.md`](AGENTS.md).

---

*AMBRE - Privacy-by-design collaboration, powered by Rust.*
