# AMBRE — SPEC: AMBRE-SYNC (SECSYNC MODULE)
**Type:** Horizontal (Layer / Engine)
**Status:** Draft

> End-to-end encrypted synchronization layer between WASM clients and the Blind Relay. This module implements the SecSync protocol in pure Rust: it transforms CRDT deltas produced by the Logic thread into cryptographically sealed messages, routes them via WebSocket, and re-injects remote updates into the Loro document — without ever exposing plaintext outside the client process.
>
> `ambre-sync` runs exclusively in the **Sync thread** (a `wasm_thread`, i.e. a Web Worker sharing linear memory). It has no access to the `LoroDoc` (Logic thread) or the WebGPU canvas (UI thread).
>
> **Model note.** The code is in **Model B** (1 WASM binary, 3 `wasm_thread` threads with shared memory, `flume` channels) — see `ambre-spec-architecture.md`. For `ambre-sync`: (1) Logic ↔ Sync communication goes through **`flume` channels** carrying **typed Rust values** (`SyncCommand`/`LogicEvent`), **no more `postMessage`/`MessagePort`/postcard** on this path; (2) the entry point is `ambre_sync::run(from_logic, to_logic)`; (3) **postcard only remains on the network wire** (`ambre-protocol`). The crypto pipeline, quarantine, batching, and security (below) are **unchanged** by this move.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:** `ambre-sync` is responsible for the entire pipeline between the Logic thread and the network. It handles: SecSync message encryption/decryption, WebSocket lifecycle management, batching and compaction, and `clock`/`seq` numbering. It does not know CRDT semantics (delegated to the Logic thread / apps), nor rendering (delegated to the UI thread).

**Dependencies:**

| Crate | Role | Rationale |
|---|---|---|
| `ambre-core` | Fundamental types (`Uuid`, `DocType`) | Common foundation |
| `ambre-crypto` | Crypto primitives (`MasterSeed`, `DocumentKey`, AEAD, sign, Argon2) | Sole crypto consumer client-side |
| `ambre-protocol` | Network types (`Update`, `Snapshot`, `Ephemeral`, `WsClientMsg`, `WsServerMsg`) | Single source of truth — no local redefinition |
| `ambre-bus` | `SyncCommand`, `LogicEvent` (+ `RestCommand`/`RestEvent`/`RequestId`, `ambre-core` re-exports) | **Typed** interface with the Logic thread — values travel by move on `flume`, **no (de)serialization**. `ambre-sync` does not depend directly on `ambre-core` (types via `ambre-bus` re-export). |
| `chacha20poly1305` (RustCrypto) | XChaCha20-Poly1305 AEAD | Via `ambre-crypto` — listed here for transitive dependency clarity |
| `ed25519-dalek` | Ed25519 signature | Via `ambre-crypto` |
| `blake2` + `hmac` + `sha2` | Hash and commitment tag | Via `ambre-crypto` |
| `ruzstd` | zstd compression AND decompression | Pure Rust, `wasm32` compatible — single crate for both directions |
| `rand_core` + `getrandom` | Nonce generation | `getrandom` with `wasm_js` feature for WASM |
| `lru` | LRU cache for `old_keys` and quarantine | Bounded, minimal allocation |
| `flume` | Logic ↔ Sync inter-thread channels | **Typed** transport (`SyncCommand`/`LogicEvent`) by move over shared memory. Entry point `run(from_logic, to_logic)`. Used in WASM **and** native (tests). |
| `postcard` | Network wire encoding | **Only** via `ambre-protocol` (WebSocket): `[version_byte] [postcard(WsServerMsg)]`. No postcard on the inter-thread path. |
| `gloo-net` (`websocket` + `http` features) | WASM WebSocket + HTTP fetch | `web_sys::WebSocket` + `web_sys::fetch` wrapped — **`!Send`, confined to the Sync thread**. (Forward-looking placeholder today; code uses `web_sys::WebSocket` directly.) |
| `web-sys` + `js-sys` + `wasm-bindgen` | `WebSocket` + async glue | The socket and its `onmessage`/`onclose` closures live in the Sync thread. |
| `wasm-bindgen-futures` | `spawn_local` for async operations | Rust future bridge → JS event loop (HTTP fetch, WebSocket connect, rotation) |
| `tokio-tungstenite` (`cfg not(wasm32)`) | Native WebSocket for tests | Headless test backend only |

> **Transport note (Model B):** `ambre-sync` receives its commands by blocking on a `flume::Receiver<SyncCommand>` (`from_logic.recv()`) and emits `LogicEvent` on a `flume::Sender<LogicEvent>` (`to_logic`). Values travel **by move** in the shared heap — no serialization. No more `postMessage`, `MessagePort`, or `SyncTransport` trait. ⚠️ **`Atomics.wait` limitation**: blocking on `flume` in a WASM Worker freezes the Worker's JS event loop (WebSocket/`setTimeout`). Tolerated while the WS pipeline is a placeholder; **once the real WebSocket is wired, the loop must become non-blocking** (`recv_async` + WS driven by `JsFuture`/`spawn_local`, WS branch disabled when `to_logic` is full). See §2.C and `ambre-spec-architecture.md §7 ARCH P1`.

> **Note on `ambre-bus`:** `ambre-sync` depends on `ambre-bus` for `SyncCommand`/`LogicEvent` (+ the `RestCommand`/`RestEvent`/`RequestId` service family) — not for `LogicCommand`, `UiEvent`, or UI types. The former `AppController`/`CommandSink` traits no longer exist.

**Invariants:**

- **Zero plaintext on wire**: no byte of the CRDT payload reaches the network unencrypted and unsigned. Statically verifiable property: `PlaintextPayload` is never passed to `WsTransport`.
- **Guaranteed causal order**: updates are always applied in `(snapshot_id, clock)` order. An update with an unknown `snapshot_id` is quarantined until the parent snapshot is received, then an automatic `WsClientMsg::RequestMissing` is triggered.
- **Idempotence**: re-applying an already integrated CRDT update has no effect (inherited Loro property). Network duplicates are tolerated.
- **No `unsafe`**: no `unsafe` in application code. RustCrypto crates are audited.
- **Nonce uniqueness**: XChaCha20 nonce (192 bits) generated via CSPRNG per message. Collision probability over 2^96 messages cryptographically negligible.
- **`WebSocket` (`web_sys`/`gloo-net`) is `!Send`**: created and used exclusively in the Sync thread.
- **The Sync thread compiles neither UI nor CRDT**: guaranteed by the crate graph (`ambre-sync` depends on neither `ambre-canvas` nor `loro`).

---

## 2. TECHNICAL DESIGN & STATE

### A. Data Modeling

Network message types are **imported from `ambre-protocol`**, not redefined here.

```rust
// Imports from ambre-protocol -- single source of truth
use ambre_protocol::{
    Update, Snapshot, Ephemeral,
    UpdatePublicData, SnapshotPublicData, EphemeralPublicData,
    WsClientMsg, WsServerMsg,
};
```

**Sync-thread-internal types (not exported):**

```rust
/// Version byte encoded in the plaintext before encryption.
/// Opaque to the server (inside the AEAD ciphertext).
#[repr(u8)]
enum CompressionVersion {
    Raw  = 0x00,
    Zstd = 0x01,
}

/// Payload before encryption. NEVER leaves this module in plaintext.
/// Manually Zeroize-able after use.
struct PlaintextPayload {
    version: CompressionVersion,
    data: Vec<u8>, // raw or compressed CRDT data
}
```

**Sync thread internal state:**

```rust
/// Global Sync thread state -- one per browser tab.
/// Manages N sync sessions in parallel (one per open document,
/// including read-only embedded documents in Toile).
/// (In code: `SyncState`.)
struct SyncState {
    /// Master Seed decrypted at login (SyncCommand::Unlock) -- ZeroizeOnDrop
    /// Sole source of signing + exchange keypairs.
    master_seed: Option<MasterSeed>,

    /// Ed25519 signing key -- derived from master_seed at login
    signing_key: Option<ed25519_dalek::SigningKey>,

    /// X25519 exchange key -- derived from master_seed at login
    exchange_key: Option<x25519_dalek::StaticSecret>,

    /// Active sync sessions -- one per open/embedded document
    documents: HashMap<Uuid, SyncDocumentState>,

    /// Single multiplexed WebSocket -- opened at Unlock, closed at Logout.
    /// All documents and channels share this connection via Subscribe/Unsubscribe.
    /// `web_sys::WebSocket` (or gloo-net), !Send, confined to the Sync thread.
    ws: Option<web_sys::WebSocket>,
    /// JS closures attached to the WebSocket (onmessage/onclose), kept alive.
    _ws_closures: Vec<Closure<dyn FnMut(MessageEvent)>>,

    /// Cache of decrypted DocumentKeys (post-MVP).
    /// Populated by OpenDocument and ListDocuments -- avoids redoing X25519 ECDH
    /// on each ListDocuments for N documents (~150us per ECDH on mobile).
    /// ZeroizeOnDrop on each entry. Invalidated on logout.
    /// MVP: no cache, ECDH recomputed on each ListDocuments (acceptable for < 20 docs).
    // doc_key_cache: HashMap<Uuid, DocumentKey>,

    /// Outbound channel to the Logic thread (typed `LogicEvent` values, by move).
    logic_tx: flume::Sender<LogicEvent>,

    /// In-memory access + refresh tokens (never localStorage/cookies). Bearer on
    /// REST, `?token=` on the WS. Holds the 401->refresh->retry logic. (Code: `AuthContext`.)
    auth: AuthContext,

    /// WS reconnection state (sec.2.G): exponential-backoff attempt counter,
    /// the buffer of edits issued before the session keys were ready
    /// (`pending_pre_session_updates`), and -- in dev -- the username to re-derive
    /// the identity on a 4401 close.
    reconnect_attempts: u32,
    pending_pre_session_updates: Vec<(DocumentId, Vec<u8>)>,
    dev_username: Option<String>,
}

/// Sync state for an individual document.
/// Created by SyncCommand::OpenDocument, destroyed by SyncCommand::CloseDocument.
struct SyncDocumentState {
    /// Current document symmetric key -- ZeroizeOnDrop
    document_key: DocumentKey,

    /// Last known snapshot (for constructing `parent_snapshot_proof`)
    current_snapshot_id:   Option<Uuid>,
    current_snapshot_hash: Option<[u8; 32]>,

    /// Local update counter (clock), strictly monotonic, never reset
    local_clock: u64,

    /// Anti-replay counter for OUTBOUND Ephemeral messages, monotonic per session
    ephemeral_counter: u64,

    /// Anti-replay for INBOUND Ephemeral messages (from other clients).
    /// Key = sender's pub_key, value = last accepted counter.
    /// An Ephemeral message with counter <= last_counter is DROPPED (replay).
    ephemeral_remote_counters: HashMap<[u8; 32], u64>,

    /// Quarantined updates: unknown parent snapshot_id -> (snapshot_id -> Vec<Update>)
    /// Capped at 1000 updates (LRU drop beyond + automatic RequestMissing)
    quarantine: BTreeMap<Uuid, Vec<Update>>,

    /// Last known global `seq` (delta-sync on reconnect)
    last_known_seq: Option<u64>,

    /// Local compaction lock (avoids double compaction from the same instance)
    compaction_lock: bool,

    /// Cache of old Document Keys post-rotation
    /// Capped at 10 entries (LRU). Key = reference snapshot_id.
    /// Eviction: oldest entry removed beyond 10.
    old_keys: lru::LruCache<Uuid, DocumentKey>,
}
```

> **No more `SyncTransport` trait.** Model A abstracted the transport (WASM MessagePort vs native flume) behind a trait. In Model B, `flume` is the transport **in both cases**: `ambre-sync` emits directly on a bare `flume::Sender<LogicEvent>` (`send_to_logic`), without abstraction.

**Execution model (Model B, coming with the real WebSocket):**

The Sync thread is a `wasm_thread` looping on `from_logic.recv()` (blocking via `Atomics.wait`). This is valid while the WebSocket is a placeholder. **When the real WS is wired**, a blocking loop would freeze the Worker's JS event loop (therefore the WS callbacks): a non-blocking `select` will be needed — `select { cmd = from_logic.recv_async() => ..., frame = ws.next() if to_logic_not_full => ... }` — where network I/O (HTTP fetch presigned URLs, rotation, WS connect) is driven by `wasm_bindgen_futures::spawn_local` / `JsFuture`. The WS branch is **disabled when `to_logic` is full** (backpressure: Sync stops reading the socket → TCP backpressure; see `ambre-spec-architecture.md §7 ARCH P1`, "cyclic deadlock").

### B. Cryptographic Pipeline

The pipeline is identical for the three message types (Snapshot, Update, Ephemeral).

**Encryption:**

```
CRDT Data (Vec<u8>)
  │
  ├─ If size > 256 bytes → ruzstd::encoding::compress_to_vec(data, CompressionLevel::Fastest)
  │                         version = 0x01
  └─ Otherwise → version = 0x00 (raw, no compression)
  │
  ▼
PlaintextPayload = [version_byte | data]
  │
  │  XChaCha20-Poly1305 (AEAD)
  │  key   = document_key (256 bits)
  │  nonce = getrandom::<[u8; 24]>()
  │  aad   = postcard::to_stdvec(&public_data)  ← binds ciphertext to metadata
  ▼
ciphertext (Poly1305 tag integrated — authenticity guaranteed)
  │
  │  HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
  │  → commitment_tag: [u8; 32]  ← key-commitment (XChaCha20-Poly1305 is not key-committing)
  │                                 Explicit field in ambre_protocol::{Update|Snapshot|Ephemeral}
  │
  │  Ed25519::sign(signing_key, nonce || ciphertext || public_data_bytes || commitment_tag)
  │  → signature [u8; 64]         ← covers commitment_tag to prevent its modification
  ▼
ambre_protocol::{Update|Snapshot|Ephemeral} ready to send via WsClientMsg
```

> **commitment_tag note:** The HMAC-SHA256 tag is distinct from the AEAD Poly1305 tag. It serves as proof of key-commitment: XChaCha20-Poly1305 is not key-committing (a given ciphertext can theoretically be decrypted by two different keys). HMAC(document_key, ...) cryptographically binds the message to a specific key, preventing a compromised server from re-injecting a ciphertext into another document context.

> **Snapshots note:** Encrypted `Snapshot` messages do not transit via WebSocket — they are uploaded/downloaded via S3 presigned URLs (see `ambre-spec-server.md` §2.B). The crypto pipeline is identical, only the transport differs.

**Decryption (symmetrical path):**

```
Reception WsServerMsg::{Update|Ephemeral|...} (or S3 blob for Snapshot)
  │
  │  1. Ed25519::verify(public_data.pub_key, nonce || ciphertext || public_data_bytes || commitment_tag)
  │     → Err(InvalidSignature) → DROP + SyncWarning
  │
  │  2. HMAC-SHA256::verify(document_key, nonce || ciphertext || public_data_bytes) == commitment_tag
  │     → Err → DROP + SyncWarning (possible inter-document substitution or wrong key)
  │
  │  3. XChaCha20-Poly1305::decrypt(document_key, nonce, aad=public_data_bytes, ciphertext)
  │     → Err(AuthenticationFailed) → DROP + SyncWarning (corruption or attack)
  ▼
PlaintextPayload = [version_byte | data]
  │
  ├─ version == 0x01 → ruzstd::decode_all(data) → Vec<u8>
  ├─ version == 0x00 → data as is
  └─ unknown version → Err(UnknownCompressionVersion) → DROP
  ▼
CRDT bytes → logic_tx.send_to_logic(LogicEvent::RemoteUpdate { .. })
```

**Key to use during decryption:**
- If `public_data.snapshot_id` corresponds to a snapshot encrypted with an old key (post-rotation), look up in `old_keys` before using `document_key`.
- If absent from `old_keys` → `Err(UnknownKey)` → quarantine + `SyncWarning`.

### C. Interface Contract (API)

**Entry point (Model B — WASM and native):**

```rust
/// Sync thread entry point. Spawned by `ambre-app` via wasm_thread (or
/// called directly in native tests). Blocking loop while WS is a
/// placeholder; to be made non-blocking when the real WebSocket is wired (cf.
/// "Atomics.wait" note above). Exits when the Logic thread disappears.
pub fn run(
    logic_rx: flume::Receiver<SyncCommand>,
    logic_tx: flume::Sender<LogicEvent>,
) {
    let mut state = SyncState::new(logic_tx, /* api_base */ String::new());
    while let Ok(cmd) = logic_rx.recv() {
        state.handle_command(cmd);
    }
    // logic_rx disconnected => Logic closed => teardown.
}

impl SyncState {
    /// Emits an event to the Logic thread (typed value, by move).
    fn send_to_logic(&self, event: LogicEvent) { let _ = self.logic_tx.send(event); }

    /// Responds to a service request by echoing its RequestId.
    fn reply(&self, id: RequestId, evt: RestEvent) {
        self.send_to_logic(LogicEvent::Rest { id, evt });
    }
}
```

**Command dispatch (two levels: envelope then service):**

`SyncCommand` falls into three groups (cf. `ambre-bus/src/command.rs`): the **service** envelope `Rest { id, cmd }` (correlatable relay), **control/content-send** for documents/channels (`OpenDocument`, `SendUpdate`, ...), and `Shutdown`. The service family is routed to `handle_rest`, which replies via `reply(id, ...)` by echoing the `id`.

```rust
impl SyncState {
    fn handle_command(&mut self, cmd: SyncCommand) {
        match cmd {
            // -- Service plane (correlatable request/response) ----------------
            SyncCommand::Rest { id, cmd } => self.handle_rest(id, cmd),

            // -- Control / content-send (document) ---------------------------
            SyncCommand::OpenDocument { document_id } => { /* GET members -> ECDH -> DocumentKey ;
                creates SyncDocumentState ; GET S3 snapshot/deltas -> RemoteSnapshot/RemoteUpdate ;
                WsClientMsg::Subscribe { document_id } */ }
            SyncCommand::CloseDocument { document_id } => { /* Unsubscribe ; zeroize DocumentKey */ }
            SyncCommand::SendUpdate { document_id, payload } => { /* compress -> encrypt -> WsClientMsg::Update */ }
            SyncCommand::SendSnapshot { document_id, payload } => { /* encrypt -> presigned S3 PUT -> POST metadata */ }
            SyncCommand::SendEphemeral { document_id, payload } => { /* encrypt -> WsClientMsg::Ephemeral */ }
            SyncCommand::Reconnect { document_id } => { /* Subscribe + RequestMissing { after_seq } */ }

            // -- Control (channel) -------------------------------------------
            SyncCommand::OpenChannel { channel_id }  => { /* SubscribeChannel */ }
            SyncCommand::CloseChannel { channel_id } => { /* UnsubscribeChannel */ }
            SyncCommand::SendChannelUpdate { channel_id, payload }    => { /* route to active segment */ }
            SyncCommand::SendChannelEphemeral { channel_id, payload } => { /* typing indicator */ }

            // -- Lifecycle ---------------------------------------------------
            SyncCommand::Shutdown => self.handle_logout(), // cleanup, zeroize, close WS
        }
    }

    /// Relayed service family. Each arm replies via `reply(id, RestEvent::...)`.
    fn handle_rest(&mut self, id: RequestId, cmd: RestCommand) {
        match cmd {
            // -- Session ----------------------------------------------------
            RestCommand::Unlock { passphrase } => {
                // spawn_local: POST /api/auth/... -> parse { access_token, refresh_token } into AuthContext
                // GET /api/keys/master + GET /api/auth/me (Bearer)
                // Argon2id(passphrase, salt) -> User Key -> decrypts master_key_enc -> MasterSeed
                // Derives signing_key + exchange_key; opens the multiplexed WS (GET /api/ws?token=<jwt>)
                // WS onmessage -> handle_ws_message ; onclose -> 4401 reauth | backoff (sec.2.G)
                //              ; onopen -> resubscribe_all ; zeroize passphrase
                // reply(id, SessionReady{..}) | AccountSetupRequired (404) | or LogicEvent::AuthError
            }
            RestCommand::SetupAccount { passphrase } => { /* ... -> reply(id, AccountCreated { recovery_words }) */ }
            RestCommand::Logout => { self.handle_logout(); self.reply(id, RestEvent::LoggedOut); }

            // -- Documents CRUD ---------------------------------------------
            RestCommand::ListDocuments => { /* GET /api/documents (doc_key_enc joined) -> ECDH -> reply(id, DocumentList) */ }
            RestCommand::CreateDocument { doc_type, name } => { /* gen DocumentKey -> POST -> reply(id, DocumentCreated{document_id, doc_type}) */ }
            // ... DeleteDocument / RenameDocument

            // -- Sharing ----------------------------------------------------
            RestCommand::InviteToDocument { document_id, invitee_email, role } => { /* search -> ECDH -> POST /share -> reply(id, InviteSent) */ }
            RestCommand::RevokeFromDocument { document_id, user_id } => {
                // atomic sequence: DELETE /share/{uid} -> gen DocumentKey -> GET members ->
                // X25519 per member -> POST /rotate-key (server broadcasts KeyRotationRequired) ->
                // swap key (old -> old_keys, zeroize) -> reply(id, RevocationComplete)
            }

            // -- Media ------------------------------------------------------
            RestCommand::UploadMedia { document_id, data, mime_type } => { /* hash -> encrypt -> presigned S3 PUT -> POST -> reply(id, MediaUploaded) */ }
            RestCommand::DownloadMedia { document_id, content_hash } => { /* presigned GET -> decrypt -> reply(id, MediaDownloaded) */ }

            // Channels (CRUD/sharing), Users, Meetings: same pattern (reply(id, ...)).
            _ => { /* ... */ }
        }
    }
}
```

> `RequestId` is **allocated by the UI**, echoed by Logic, then echoed by Sync in the response — `ambre-sync` merely re-emits it via `reply`. **Errors** are not in `RestEvent`: they arrive as `LogicEvent::AuthError`/`ApiError` (may be response **or** spontaneous).

`SyncCommand`/`LogicEvent` types (+ `RestCommand`/`RestEvent`/`RequestId`) are defined in `ambre-bus` and imported here — see `ambre-spec-architecture.md §2.D` (typed data plane, three planes, and business types `DocumentSummary`/`MemberInfo`/`UserInfo`/...).

### D. Data Flow

```
Logic Thread                        Sync Thread                    Blind Relay / S3
─────────────────                  ──────────────────────          ─────────────────
     │                                      │                           │
     │── SyncCommand::Unlock ──────────────▶│                           │
     │                                      │── GET /keys/master ──────▶│
     │                                      │── GET /auth/me ──────────▶│
     │                                      │── Argon2 + decryption     │
     │                                      │── opens WS /api/ws ──────▶│
     │◀─ LogicEvent::SessionReady ─────────│                           │
     │                                      │                           │
     │── SyncCommand::ListDocuments ───────▶│── GET /documents ────────▶│
     │                                      │── decrypts name_enc       │
     │◀─ LogicEvent::DocumentList ─────────│                           │
     │                                      │                           │
     │── SyncCommand::OpenDocument ────────▶│── GET /members ──────────▶│
     │                                      │── decrypts DocumentKey    │
     │                                      │── GET S3 snapshot ───────▶│
     │                                      │── Subscribe {doc_id} ────▶│
     │◀─ LogicEvent::RemoteSnapshot ───────│                           │
     │◀─ LogicEvent::ConnectionStatus ─────│                           │
     │                                      │                           │
     │── SyncCommand::SendUpdate ──────────▶│── compress → encrypt ────▶│
     │                                      │── WsClientMsg::Update     │
     │                                      │◀─ WsServerMsg::Update ────│
     │◀─ LogicEvent::RemoteUpdate ─────────│                           │
     │                                      │                           │
     │── SyncCommand::SendSnapshot ────────▶│── encrypt → S3 PUT ──────▶│
     │                                      │── POST metadata ─────────▶│
     │                                      │                           │
     │── SyncCommand::InviteToDocument ────▶│── GET /users/search ─────▶│
     │                                      │── encrypts DocumentKey    │
     │                                      │── POST /share ───────────▶│
     │◀─ LogicEvent::InviteSent ───────────│                           │
     │                                      │                           │
     │── SyncCommand::RevokeFromDocument ──▶│── DELETE /share/{uid} ───▶│
     │                                      │── generates new key       │
     │                                      │── POST /rotate-key ──────▶│
     │◀─ LogicEvent::RevocationComplete ───│                           │
     │                                      │                           │
     │── SyncCommand::UploadMedia ─────────▶│── encrypt → S3 PUT ──────▶│
     │◀─ LogicEvent::MediaUploaded ────────│                           │
```

### E. Quarantine Management

An update with an unknown parent `snapshot_id` is quarantined:

```
Reception Update with unknown snapshot_id
  │
  ├─ Add to quarantine[snapshot_id].push(update)
  │
  ├─ If quarantine.total_len() >= 900 (alert threshold)
  │   └─ Send WsClientMsg::RequestMissing { after_seq: last_known_seq }
  │
  ├─ If quarantine.total_len() >= 1000 (hard limit)
  │   └─ Drop oldest updates (LRU by clock)
  │   └─ LogicEvent::SyncWarning { code: QUARANTINE_OVERFLOW }
  │
  Reception Snapshot with matching snapshot_id
  │
  └─ Release quarantine[snapshot_id] in clock ASC order
      └─ Apply each update via LogicEvent::RemoteUpdate
```

> **Genesis snapshot id (never-snapshotted documents).** A brand-new document has no snapshot, yet its first deltas still need a stable parent `snapshot_id` or they would quarantine forever (random per-client id, never uploaded). Every client deterministically derives the same base id from the document id: `SnapshotId::genesis(document_id) = SnapshotId::from_bytes(*document_id.as_bytes())` (pure client-side, `ambre-core`). `SyncDocumentState::new` and `handle_open_document` seed `known_snapshots` with this genesis id, so the very first deltas are always releasable across F5 and across devices. (Deltas written before this rule, under an old random id, are orphans -- only relevant to pre-existing dev data.)

### F. Batching and Compaction

```
Reception SyncCommand::SendUpdate
  │
  ├─ Immediate encryption → WsClientMsg::Update sent (real-time)
  └─ Increment local_clock

Reception SyncCommand::SendSnapshot
  │
  ├─ Verify compaction_lock == false
  │   └─ If true → LogicEvent::SyncWarning { code: COMPACTION_IN_PROGRESS } → skip
  │
  ├─ compaction_lock = true
  ├─ Full snapshot encryption
  ├─ POST to Object Storage (via presigned URL from the Blind Relay)
  ├─ Signal server: purge deltas seq <= current_seq
  └─ compaction_lock = false
```

### G. Authentication & WebSocket Resilience

The server uses **no cookie**: a stateless EdDSA access JWT (TTL 900s) + an opaque revocable refresh token (see `ambre-spec-server.md §3`). The Sync worker is the sole holder of both.

**Token store (`AuthContext`).** Access + refresh tokens are held **in memory** in the Sync worker (never `localStorage`/cookies). `Unlock`/`SetupAccount`/`dev-login` parse the JSON login body (`{ access_token, refresh_token, access_expires_in }`) into the store. Every REST call sends `Authorization: Bearer <access_token>`; the WebSocket carries the access token in the **`?token=<jwt>`** query parameter (browser WS forbids custom headers).

**REST 401 -> refresh -> retry.** A thin wrapper around `gloo-net::http` retries once on `401`: `POST /api/auth/refresh { refresh_token }` -> store the rotated pair -> replay the original request. A second `401` (or a refresh failure) surfaces as `LogicEvent::AuthError` (the UI must re-authenticate).

**WebSocket reconnection.** The single multiplexed WS is opened at `Unlock` and must survive token expiry, server restarts (ephemeral JWT seed in dev), and network blips:

```
onclose(code) ─┬─ code == 4401 (unauthorized) ─► reauthenticate():
               │     refresh().await  (dev: re-run dev-login from the stored username)
               │       ok  -> reset backoff, reopen WS
               │       err -> schedule_reconnect()
               │
               └─ any other close ─────────────► schedule_reconnect()

schedule_reconnect(): exponential backoff base 250ms x2, cap 10s, + random jitter,
                      armed via setTimeout (DedicatedWorkerGlobalScope) -> ReconnectNow.
                      The attempt counter is reset ONLY on a real inbound frame
                      (not on bare onopen: an auth-rejected upgrade emits 101 then an
                      immediate 4401 close -- resetting on onopen would tight-loop and
                      saturate the WS rate-limit -> 429).

onopen ──► resubscribe_all(): for each server-created doc, ws_send(Subscribe) +
           ws_send(RequestMissing { after_seq: last_seq }). MUST run on onopen, not
           synchronously in open_websocket: a Subscribe sent while the socket is
           CONNECTING is dropped -> server has the connection but no subscription
           -> 403 "not subscribed".
```

> **Decisive rationale (4401 vs generic drop).** When the access token expires or the server rotates its (dev-ephemeral) JWT seed, every token is invalid. A generic close would trigger plain backoff that never recovers (the new connection is rejected too). The explicit **4401** lets the client refresh (or, in dev, re-derive its identity via dev-login) and reconnect **silently**, with no user action.

**Pre-session update buffer.** An edit issued while `Unlock` is still in flight (`signing_key` is `None`) is **not dropped**: it is queued in `pending_pre_session_updates` and flushed by `flush_pre_session_updates(doc)` -- for a server-created doc after `MissingUpdates` (the `RequestMissing` replay has caught `local_clock` up), for a local-only doc at `OpenDocumentReady`. Dropping it instead caused the document's first edit to never reach the server (CLOCK_MISMATCH on the next edit).

**Self-echo clock recovery.** `local_clock` is not persisted across refresh (F5). On reconnect the server echoes the client's own past `Update`s; `receive_update` bumps `local_clock = max(local_clock, update.clock)` when the update's author key is ours, so the next outbound edit picks up at the right clock instead of re-emitting an already-consumed value (which the Lua script rejects with CLOCK_MISMATCH).

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

### Boundary Audit

| Data | Leaves the client? | Wire form | Guarantee |
|---|---|---|---|
| CRDT content (text, data) | Yes | XChaCha20-Poly1305 ciphertext | Blind Relay sees `ciphertext` + `nonce`, never plaintext |
| Document Key | No | Stays in `SyncState` — ZeroizeOnDrop | Never in a network message |
| Signing Key (private) | No | Same | Only `pub_key` in `public_data` |
| Compression version | No | Encoded in AEAD ciphertext | Opaque to the server |
| `document_id`, `pub_key`, `clock`, `seq` | Yes | Plaintext in `public_data` | Minimal routing metadata — does not reveal content |
| Cursors / presence | Yes | Encrypted in `Ephemeral` payload | Server never sees cursor or name in plaintext |

**Accepted metadata leak (MVP):** the Blind Relay infers "a client with public key X modified document Y at instant T". This information is deemed acceptable for MVP.

### Attack Prevention

- **Replay (Updates)**: the per-client `clock` is checked sequentially by the server (Redis Lua script). An update with `clock <= last_known_clock` is rejected server-side.
- **Replay (Ephemeral)**: the `counter` is monotonic per `(document_id, session)`. A receiving client rejects any Ephemeral message with `counter <= last_known_counter` for that `(pub_key, document_id)`.
- **Inter-document substitution**: the `commitment_tag` HMAC(document_key, nonce || ciphertext || public_data) binds the ciphertext to its `document_id`. A message cannot be re-injected into another document.
- **Snapshot reordering**: the `parent_snapshot_proof` (BLAKE2b chain) guarantees chain integrity without decryption.
- **Double Compaction**: Redis lock `SET NX TTL 30s` server-side + `compaction_lock` client-side.
- **Plaintext presence**: Ephemeral payloads (cursor, selection, name) are **encrypted** with the Document Key before sending. The server only sees `document_id`, `pub_key`, `counter`.

### Document Key Rotation — Full Flow

Two triggers: (A) the initiator client revokes a member via `SyncCommand::RevokeFromDocument`, (B) other clients receive `WsServerMsg::KeyRotationRequired` via WebSocket.

Rotation is **entirely orchestrated by the Sync thread** — the Logic thread does neither crypto nor network. The DocumentKey never leaves the Sync thread.

```
[Initiator client — Logic thread]
1. User revokes a member via UI
   Logic thread emits SyncCommand::RevokeFromDocument { document_id, user_id }

[Initiator client — Sync thread (async via spawn_local)]
2. DELETE /api/documents/{id}/share/{uid}
3. Generates new DocumentKey via CSPRNG
4. GET /api/documents/{id}/members → list of remaining members + pubkeys
5. For each remaining member:
   X25519(own_exchange_key, member.exchange_pubkey) → encrypts new key
6. POST /api/documents/{id}/rotate-key with encrypted_keys_per_member
   → server atomically replaces all doc_key_enc (Postgres transaction)
   → server broadcasts WsServerMsg::KeyRotationRequired at THIS moment
     (new keys are in the database — no race condition)
7. Completes messages being encrypted with the old key
8. Stores old key in old_keys (LRU, max 10 entries)
9. Replaces document_key with new_key, zeroize() the old one
10. Sends LogicEvent::RevocationComplete { document_id } to the Logic thread

[Initiator client — Logic thread]
11. Triggers SyncCommand::SendSnapshot { document_id, payload }
    → snapshot encrypted with the new key → seals post-revocation state

[Other clients — Sync thread (receiving WsServerMsg::KeyRotationRequired)]
12. GET /api/documents/{id}/members → fetches new doc_key_enc
13. X25519 ECDH → decrypts new DocumentKey
14. Swaps document_key, old → old_keys, zeroize
15. Sends LogicEvent::MemberRevoked { document_id, revoked_user_id } to the Logic thread
    (Logic thread is informed of the departure, can update UI)
```

**Security:** The DocumentKey never crosses the `flume` Logic ↔ Sync channel (stays confined to the Sync thread). The `WsServerMsg::KeyRotationRequired` broadcast is emitted by the server **after** new keys are in the database (step 6) — other clients never fetch a stale `doc_key_enc`.

**`old_keys` cache:** capped at 10 LRU entries per document. Beyond that, snapshots prior to the 11th rotation are no longer locally decryptable → force a re-sync from S3.

---

## 4. QUALITY & TESTABILITY

### Unit Tests

```rust
// Round-trip encrypt/decrypt for all three message types
#[test]
fn roundtrip_update() { /* encrypt -> decrypt, assert_eq payload */ }

// Tampering detection
#[test]
fn tampered_ciphertext_fails() {
    let mut enc = encrypt_update(&data, &key, &nonce, &public_data);
    enc.ciphertext[0] ^= 0xFF;
    assert!(decrypt_update(&enc, &key).is_err());
}

// HMAC commitment: inter-document substitution detected
#[test]
fn commitment_tag_wrong_document_rejected() { /* modify document_id -> invalid HMAC */ }

// Invalid signature
#[test]
fn invalid_signature_rejected() { /* wrong pub_key -> Err(InvalidSignature) */ }

// Version byte
#[test]
fn version_byte_routing() {
    // 0x00 -> raw, 0x01 -> decompress, 0xFF -> Err(UnknownCompressionVersion)
}

// Quarantine: ordered release
#[test]
fn quarantine_released_in_clock_order() { /* update before snapshot -> quarantine -> snapshot -> release */ }

// Quarantine: RequestMissing triggered at 900
#[test]
fn quarantine_triggers_request_missing_at_threshold() { }

// Anti-replay Ephemeral
#[test]
fn ephemeral_counter_replay_rejected() { /* counter <= last known -> DROP */ }
```

### Prop-Testing

```rust
proptest! {
    /// Property: encrypt(decrypt(m)) == m for any payload and any key.
    #[test]
    fn roundtrip_encrypt_decrypt(
        data in any::<Vec<u8>>(),
        key  in any::<[u8; 32]>(),
    ) {
        let encrypted = encrypt_update(&data, &key, /* ... */).unwrap();
        let decrypted = decrypt_update(&encrypted, &key).unwrap();
        prop_assert_eq!(data, decrypted);
    }

    /// Property: clocks are strictly increasing.
    #[test]
    fn clock_monotonically_increases(n_updates in 1usize..1000) {
        let mut state = SyncState::new_for_test();
        let clocks: Vec<u64> = (0..n_updates)
            .map(|_| state.next_clock())
            .collect();
        // windows(2) returns &[u64] -- no tuple
        prop_assert!(clocks.windows(2).all(|w| w[0] < w[1]));
    }

    /// Property: compression then decompression is identity.
    #[test]
    fn ruzstd_roundtrip(data in any::<Vec<u8>>()) {
        use ruzstd::encoding::{compress_to_vec, CompressionLevel};
        use ruzstd::decoding::StreamingDecoder;
        let compressed = compress_to_vec(&data, CompressionLevel::Fastest);
        let mut decoder = StreamingDecoder::new(compressed.as_slice()).unwrap();
        let mut decompressed = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut decompressed).unwrap();
        prop_assert_eq!(data, decompressed);
    }
}
```

### Mocks & Environment

```rust
/// In-memory WebSocket transport -- bidirectional flume channel.
/// Allows testing the full pipeline without WASM or network.
pub struct MockWsTransport {
    pub client_tx: flume::Sender<WsClientMsg>,
    pub server_rx: flume::Receiver<WsClientMsg>,
    pub server_tx: flume::Sender<WsServerMsg>,
    pub client_rx: flume::Receiver<WsServerMsg>,
}

// The Logic <-> Sync channel IS already `flume` in Model B: no transport mock
// needed. A test builds `SyncState` with a (Sender, Receiver) flume pair
// and drains `Receiver<LogicEvent>` to observe responses (same approach as
// the 9 host tests of `ambre-logic`, cf. ambre-spec-architecture.md sec.4).

/// In-process Blind Relay for multi-client integration tests.
/// Simulates: createSnapshot, createUpdate, getDocument, hasAccess, hasBroadcastAccess.
pub struct MockBlindRelay { /* ... */ }
```

All tests compile in native mode (`#[cfg(not(target_arch = "wasm32"))]`) with `tokio::test`. WASM tests are reserved for real `gloo-net` interactions and run with `wasm-bindgen-test`.

### Benchmarks (criterion)

| Benchmark | Target (P95 mobile) | Ceiling (never exceed) | Note |
|---|---|---|---|
| Outbound encryption 1 KB (compress+AEAD+HMAC+Ed25519 sign) | < 200 us | < 500 us | Bottleneck: Ed25519 sign (~80-150 us) |
| Inbound decryption 1 KB (Ed25519 verify+HMAC+AEAD+decompress) | < 350 us | < 1 ms | Bottleneck: Ed25519 verify (~150-250 us) |
| Full crypto round-trip (excluding network) | < 600 us | < 1.5 ms | |
| zstd compression snapshot 100 KB (`ruzstd`) | < 5 ms | — | |
| Quarantine release 100 updates | < 1 ms | — | |

> **Architecture alignment:** these targets are identical to `ambre-spec-architecture.md` §5. P95 targets are calibrated on RustCrypto WASM benchmarks (without SIMD) on mid-range 2024 mobile SoC. The `ambre-crypto` benchmark (§7 SYNC P0) validates these targets on real mobile device.

---

## 5. PERFORMANCE & RESOURCES

**Performance budget:**

- The Sync thread is a dedicated `wasm_thread` (Web Worker sharing linear memory): it executes in **true parallelism** from the Logic thread and the UI thread, on its own core if available.
- Crypto operations are blocking in the Sync thread but affect neither the Logic thread nor the UI thread (separate threads, separate event loops). `flume` transport moves payloads **by move** in the shared heap — no boundary recopy.
- Target latency for a 1 KB update: outbound encryption < 200 us (P95 mobile), ceiling < 500 us.

**Binary size (`ambre-sync` contribution to total binary):**

| Component | Estimated raw | Notes |
|---|---|---|
| `ambre-sync` logic | ~50 KB | Excluding dependencies |
| RustCrypto (chacha20 + ed25519 + blake2 + hmac + argon2) | ~140 KB | Shared with `ambre-crypto` (Argon2 added for login) |
| `ruzstd` (compression + decompression) | ~80 KB | |
| `gloo-net` (websocket + http) | ~40 KB | |
| `postcard` | ~15 KB | Shared |
| **Total contribution** | **~325 KB raw** | Shared dependencies deduplicated in final binary |

**Memory (`SyncState` multi-document):**

- Base (session): < 1 MB (MasterSeed, SigningKey, ExchangeKey + HashMap overhead)
- Per open document (`SyncDocumentState`): < 1 MB (DocumentKey, counters, sync state, quarantine)
- `quarantine` per document: capped at 1000 updates × ~2 KB average = ~2 MB max
- `old_keys` per document: capped at 10 × DocumentKey (32 bytes + overhead) = negligible
- Typical scenario (1 host doc + 3 embedded): ~4-8 MB total Sync overhead

---

## 6. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|---|---|
| **libsodium / sodiumoxide** | C dependency, incompatible `wasm32-unknown-unknown` without shim. RustCrypto is equivalent, audited, pure Rust. |
| **TLS as sole security layer** | TLS protects transport but the server would see plaintext. Application E2EE is non-negotiable. TLS is complementary. |
| **`zstd` crate (C bindings) for compression** | C dependency not WASM-compatible. `ruzstd` provides compression AND decompression in pure Rust since v0.8. |
| **Dual feature flag `ruzstd` (decompression) + `zstd` (native compression)** | Unnecessary complexity. `ruzstd` covers both directions with acceptable WASM mobile performance. |
| **Signal Protocol (Double Ratchet)** | Designed for 1-to-1 messaging with per-message forward secrecy. Our model is document-based (N collaborators, shared Document Key). MLS V2. |
| **WebRTC DataChannel (P2P)** | Requires STUN/TURN, complicates NAT traversal, incompatible with Blind Relay model. Marginal latency gain for document use. |
| **JSON on the internal wire** | `postcard` is ~10x more compact, faster parsing, already in the project. |
| **Unbounded quarantine** | A permanently missing snapshot would make quarantine grow without limit. LRU + automatic RequestMissing at 900 updates is the right balance. |
| **Unbounded `old_keys` cache** | On rapid repeated rotations (attack or bug), the cache would grow. LRU at 10 entries is sufficient for real cases (< 5 simultaneously active rotations). |
| **Wire types redefined in `ambre-sync`** | `ambre-protocol` is the source of truth. Redefining locally (`SecSyncUpdate`, etc.) creates guaranteed divergence (e.g. `[u8; 32]` vs `Vec<u8>` for `pub_key`). |
| **Yrs instead of Loro** | Out of scope. Yrs and Loro snapshots have comparable compression ratios (3:1 to 8:1 on text). The Loro choice imposes no constraint on `ambre-sync`. |
| **Model A — shared-nothing (`postMessage` + `MessagePort` + postcard for Logic ↔ Sync)** | ⚠️ **v1 reversal.** Model A imposed `postcard` + 2 WASM↔JS memcpy per hop and a `SyncTransport` trait. Replaced by **`flume` channels** carrying typed Rust values by move (shared memory). Do not go back: we would reintroduce serialization and transport abstraction for nothing. The accepted cost of Model B (nightly toolchain `-Z build-std`, COOP/COEP headers for `SharedArrayBuffer`) is documented in `ambre-spec-architecture.md §6`. |
| **`wasm_thread::spawn` + `SharedArrayBuffer` — ADOPTED (was rejected in v1)** | Now **adopted**: the Sync thread is a `wasm_thread` sharing linear memory. Requires `+atomics,+bulk-memory,+mutable-globals` + `-Z build-std` + nightly + COOP/COEP headers — accepted to benefit from copy-free typed `flume` transport. |
| **One Sync thread per open document** | Would imply N additional threads for a Toile with embedded documents — each with its own RustCrypto copy. Unacceptable on mobile. Single multi-document Sync thread (`HashMap<Uuid, SyncDocumentState>`) with a single multiplexed WebSocket (Subscribe/Unsubscribe per document). |
| **Blocking `flume` loop with a real WebSocket** | `Atomics.wait` (under `flume::recv`) freezes the Worker's JS event loop, therefore the WS callbacks. To avoid once the real WS is wired: non-blocking loop `select { recv_async ..., ws.next() ... }`. Detail in `ambre-spec-architecture.md §7 ARCH P1`. |
| **One WebSocket per open document** | Connection waste (5 WS for a Toile with 4 embedded documents). Browser limits (~6 connections per origin), keepalives costly on mobile battery. Messages already carry `document_id` in `publicData` — the server demultiplexes. Single multiplexed WS per session with Subscribe/Unsubscribe. |
| **Snapshot via WebSocket** | Snapshots can be multiple MB — too large for a WebSocket message. Upload/download via S3 presigned URLs. `WsServerMsg::SnapshotAvailable` notifies clients. |

---

## 7. INTEGRATION PLAN (ROADMAP)

> Detailed backlog of this spec; cross-cutting scheduling (milestones) lives in `specs/ambre-roadmap.md`.

### SYNC P0 — Core Crypto (standalone)

- [x] Encrypt/decrypt pipeline for the 3 message types (with `commitment_tag` HMAC), in complete isolation. *`ambre-crypto`: generic `seal`/`open` envelope over `aad` (the 3 types only differ by their serialized `PublicData`); AEAD / HMAC-SHA256 / Ed25519 / X25519 / Argon2id / BLAKE2b primitives.*
- [x] Exhaustive unit tests + prop-tests (round-trip, tampering, commitment tag). *The `version byte` stays at the Sync layer (compression), outside `ambre-crypto`.*
- [x] Reference criterion benchmarks. *Desktop: seal 1 KB ~34 us, open 1 KB ~48 us (P95 targets 200/350 us). Mobile measurement to be done at Sync wiring.*
- [x] `ruzstd` compress/decompress roundtrip validation *(Sync layer: `ambre-sync::pipeline`, version-byte 0x00/0x01, threshold 256 bytes, host tests)*
- [x] `MasterSeed` → `derive_signing_key` / `derive_exchange_key` implementation + tests *(domain-separated BLAKE2b derivation, deterministic)*
- [x] No WebSocket, no Logic thread *(standalone crate, zero network/IPC dep)*

### SYNC P1 — Logic thread integration

- [x] Entry point `ambre_sync::run(from_logic, to_logic)` (`flume` channels), blocking `recv()` loop, `SyncState` skeleton + `handle_command`/`handle_rest` dispatch (placeholders).
- [x] **Wired crypto pipeline** (`ambre-sync::pipeline`): compression version-byte + `ambre-crypto::envelope`, `Update`/`Snapshot`/`Ephemeral` assembly/disassembly, outbound encryption (`SendUpdate`/`SendEphemeral` → `WsClientMsg`, Snapshot seal/S3 TODO) and inbound decryption (`handle_ws_message` → `RemoteUpdate`/`RemoteEphemeral`, failure → `SyncWarning`). Session keys derived from `MasterSeed` (local placeholder seed pending network Unlock). Host tests. *Real network/WebSocket out of scope (next item).*
- [x] **Non-blocking loop** (`recv_async` + WS `JsFuture`) when the real WebSocket is wired — prerequisite `Atomics.wait` + cyclic deadlock fix (cf. `ambre-spec-architecture.md §7 ARCH P1`).
- [x] `RestCommand::Unlock` implementation (fetch GET /api/keys/master + Argon2 → MasterSeed → keypair derivation)
- [x] `RestCommand::SetupAccount` implementation (MasterSeed generation, key upload)
- [x] `SyncCommand::OpenDocument` / `CloseDocument` implementation (multi-document `HashMap<Uuid, SyncDocumentState>`)
- [x] Documents CRUD implementation (`RestCommand`): `ListDocuments`, `CreateDocument`, `RenameDocument`, `DeleteDocument`
- [x] `MockWsTransport` for end-to-end tests without network
- [x] Quarantine: implementation + ordered release tests + automatic RequestMissing *(release by `clock` ASC on `SnapshotAvailable`, cap 900/1000 + `SyncWarning` on overflow, 4 host tests)*

### SYNC P2 — Blind Relay & Network

- [x] Real WebSocket connection (`gloo-net` WASM, `tokio-tungstenite` native)
- [x] HTTP REST via `gloo-net::http` (CRUD, sharing, media, users search)
- [x] Delta-sync on reconnect (`last_known_seq` → `RequestMissing`)
- [x] `WsServerMsg::SnapshotRequired` handling → `LogicEvent::SnapshotNeeded`
- [x] `MockBlindRelay` in-process for multi-client tests
- [x] Auth: `AuthContext` (in-memory access + refresh tokens), `Authorization: Bearer` on REST, `?token=` on the WS, 401 -> `/auth/refresh` -> retry wrapper.
- [x] WS resilience (sec.2.G): exponential-backoff reconnect (250ms..10s + jitter), close **4401** -> silent reauth (refresh, or dev re-login), resubscribe + `RequestMissing` on `onopen`, pre-session update buffer, self-echo `local_clock` recovery.
- [x] Hydration across refresh/devices: deterministic `SnapshotId::genesis(document_id)` anchors a never-snapshotted document's first deltas (no eternal quarantine).

### SYNC P3 — Sharing & Revocation

- [x] Full `InviteToDocument` flow (search user → fetch pubkeys → ECDH wrap DocumentKey → POST /share)
- [x] Full `RevokeFromDocument` flow (DELETE /share → key rotation → POST /rotate-key → 15 steps, see §3)
- [x] LRU `old_keys` cache, 10 entries per document
- [x] Ephemeral anti-replay (`ephemeral_remote_counters`)
- [x] Media flow: `UploadMedia` (encryption + presigned URL + S3 PUT + confirmation POST), `DownloadMedia` (S3 GET + decryption)

### SYNC P4 — Channels, Meetings & Optimization

- [ ] Murmure channels: `OpenChannel`, `SendChannelUpdate`, `SendChannelEphemeral`, internal segment management (creation, sealing, routing — the Logic thread does not see segment_ids)
- [ ] Meetings: WebRTC signaling (`SignalingClientMsg` / `SignalingServerMsg`)
- [ ] Compression threshold tuning (default 256 bytes — adjust if benchmarks justify)
- [ ] WebSocket backpressure: bounded pending message buffer
- [ ] Security audit: ZK boundary review, systematic `zeroize` verification on all sensitive types

### SYNC Post-MVP — Optimizations

- [ ] `doc_key_cache: HashMap<Uuid, DocumentKey>` in `SyncState` — avoids N × X25519 ECDH on each `ListDocuments` (populated by `OpenDocument` and `ListDocuments`, zeroize on logout)
