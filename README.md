# AMBRE - E2EE collaborative office suite, 100% Rust.

**End-to-End Encrypted, 100% Rust**

AMBRE is a next-generation collaborative office suite built entirely in Rust, featuring real-time collaboration with CRDTs, end-to-end encryption (E2EE), and GPU-accelerated rendering via wgpu (WebGL2/WebGPU). It runs as a single-page WebAssembly application with a Rust-native server backend.

## Vision & Identity

AMBRE is a collaborative office suite that is:
- **100% Rust** – both client (WASM) and server
- **End-to-End Encrypted (Zero-Knowledge)** – the server acts as a blind relay
- **Open Source** – fully transparent development

**Differentiator:** The only suite combining CRDT + Rust/WASM/wgpu + native E2EE without privacy compromises.

## Applications

| Application | Description |
|-------------|-------------|
| **Prose** | Collaborative word processing |
| **Trame** | Collaborative spreadsheet |
| **Toile** | Infinite whiteboard and presentations |
| **Murmure** | Secure messaging with E2EE channels |
| **Rendez-Vous** | E2EE video conferencing (WebRTC/SFrame) |

## Core Principles (Non-Negotiable Pillars)

| Pillar | Constraint |
|--------|------------|
| **Rust-first** | 100% Rust client and server. Pure Rust libraries preferred over C bindings. |
| **Pure CSR** | Single Page Application in WASM. All computation client-side; server only relays. |
| **E2EE** | Server is a blind relay – never sees plaintext content. |
| **Security** | No `unsafe` in our code. TOFU (Trust On First Use) model with out-of-band verification. |
| **Performance** | Mobile-first targets (see below). |
| **Zero-Knowledge** | Server only knows routing metadata (document_id, public key, clock, sequence). No content, no encryption keys. |

## Performance Targets (Mobile-First)

All targets calibrated for a mid‑range 2024 smartphone.

| Requirement | Target | Justification |
|-------------|--------|---------------|
| **Render framerate** | 60+ FPS | Smooth interaction, no jank. Render thread must never be blocked by computation. |
| **Content display latency** | < 15 ms | Between user action (scroll, input) and visual update. Users should perceive no lag. |
| **Inter‑user latency** | < 300 ms | Between one user’s input and display on a collaborator’s screen. Network‑dependent, but client processing adds ≤ 50 ms. |
| **Initial load time** | < 3 s on 4G | First interactive display. WASM binary < 2 MB gzipped. |
| **Client memory** | < 100 MB | For a typical document. Proportional to document size, not its history. |

**Architectural orientation:** 3 isolated WASM workers (render / business logic / encrypted sync), GPU rendering via wgpu (WebGL2 default, WebGPU opt-in), real‑time CRDT collaboration.

## Technology Choices

| Domain | Choice | Justification |
|--------|--------|---------------|
| **Language** | Rust → WebAssembly (client), native Rust (server) | Memory safety without GC, performance, compiler as safety net for AI‑generated code. |
| **Rendering** | wgpu + lyon + Parley/Swash | Vertex/fragment shaders only (no compute overhead). WebGL2 by default, WebGPU opt-in. Path tessellation via lyon, text via Parley/Swash glyph atlas. No DOM HTML for application UI. |
| **Collaboration** | CRDT | Conflict resolution without a central truth server. Compatible with E2EE (no server content understanding). Offline‑first editing. |
| **Internal serialization** | Compact binary format | IPC between workers and network protocol. Small, fast to parse, no textual overhead. JSON only for REST API. |
| **Real‑time transport** | Binary WebSocket | CRDT deltas, presence, cursors. Persistent connection, low per‑message overhead. |
| **HTTP transport** | HTTP/2 minimum, HTTP/3 (QUIC) targeted | Multiplexing (HTTP/2), reduced mobile latency (QUIC eliminates TCP head‑of‑line blocking). |
| **Encryption** | Modern pure‑Rust primitives | AEAD (XChaCha20‑Poly1305), signatures (Ed25519), key exchange (X25519), hash (BLAKE2b), KDF (Argon2). No OpenSSL bindings. |
| **Compression** | zstd, pure Rust implementation | Applied before encryption. WASM‑compatible. |

## Project Structure

AMBRE is organized as a Cargo workspace with the following crates:

| Crate | Purpose |
|-------|---------|
| `ambre-core` | Core types, UUID, versioning, errors |
| `ambre-crypto` | Cryptographic primitives (AEAD, signatures, KDF) |
| `ambre-protocol` | Wire format, WebSocket messages, signaling |
| `ambre-bus` | Inter‑worker communication |
| `ambre-draw` | `DrawSurface` trait + shared primitives (kurbo, peniko, parley types) |
| `ambre-sync` | CRDT synchronization layer |
| `ambre-server` | Blind‑relay server (Axum, PostgreSQL, Redis) |
| `ambre-trame` | Spreadsheet application |
| `ambre-prose` | Word processor application |
| `ambre-toile` | Whiteboard/presentation application |
| `ambre-murmure` | Secure messaging |
| `ambre-app` | Main WASM application (UI integration) |

## Getting Started

### Prerequisites

- Rust stable (with `wasm32-unknown-unknown` target)
- `trunk` for building and serving the WASM frontend
- PostgreSQL and Redis (for the server)

### Building

```bash
# Install wasm target
rustup target add wasm32-unknown-unknown

# Build all crates
cargo build --release

# Build the WASM application
cd crates/ambre-app
trunk build --release
```

### Running the Server

```bash
cd crates/ambre-server
cargo run --release
```

The server will start on `http://localhost:3000`. The frontend can be served via `trunk serve` (default port `8080`).

## Roadmap

See [`specs/ambre-roadmap.md`](specs/ambre-roadmap.md) for detailed milestones and release planning.

## License

AMBRE is open‑source software licensed under the [GNU Affero General Public License v3.0 (AGPL-3.0)](LICENSE).

This license ensures that:
- The software remains free and open-source
- Any modifications to the software must be shared with the community
- Network services using AMBRE must make their source code available to users

See the `LICENSE` file for full terms and conditions.

## Contributing

Contributions are welcome! Please read our [Contributing Guidelines](CONTRIBUTING.md) (to be written) and adhere to the project’s coding standards and architectural principles.

---

*AMBRE – Privacy‑by‑design collaboration, powered by Rust.*
