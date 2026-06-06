# AMBRE — SPEC: Server (Blind Relay)
**Type:** Horizontal (Layer/Engine)
**Status:** Draft

> The AMBRE server is a **Blind Relay**: it routes, sequences, and persists encrypted blobs without ever accessing plaintext content. It exposes a REST API + WebSocket via Axum (Tokio), serialized in Postcard (binary) for real-time messages. It makes no business decisions on content — only metadata, ACLs, and sequencing are visible to it.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:** The server exclusively handles:
- OIDC authentication and session management
- Metadata storage and retrieval (PostgreSQL)
- The ephemeral and real-time layer (Redis/Valkey)
- Routing and sequencing of encrypted blobs to/from Object Storage (S3)
- Generation of presigned URLs for direct client → S3 access
- WebRTC signaling relay (meetings)
- Inter-client WebSocket broadcast via Redis PubSub

It does **not** handle:
- Content encryption/decryption (Sync WASM thread domain)
- CRDT logic (Logic WASM thread domain)
- Cryptographic key management in plaintext (never transmitted to the server)
- Content compression (opaque — version byte in ciphertext)

**Dependencies:**

| Crate | Rationale |
|---|---|
| `axum` | Mature async HTTP framework, native Tokio integration |
| `tokio` | Async runtime |
| `sqlx` | Compile-time verified SQL queries, native async |
| `rustis` | Multiplexed async Redis client (RESP3, Clone-able `Client` without pool) |
| `aws-sdk-s3` | Official S3 SDK for presigned URLs |
| `openidconnect` | OIDC Rust client, PKCE flows |
| `ed25519-dalek` | Ed25519 signature verification on updates — **server's only crypto operation** |
| `postcard` | Compact binary serialization for WebSocket messages |
| `serde` / `serde_json` | REST serialization (JSON) |
| `uuid` | UUIDv7 for identifiers |
| `tracing` + `tracing-subscriber` | Structured logging, Loki export via Promtail |
| `thiserror` | Typed errors mapped to HTTP |
| `tower` + `tower-http` | Middlewares (CORS, compression, timeout, rate limiting) |
| `axum-extra` | Cookie jar, typed headers |
| `jsonwebtoken` | JWT OIDC validation |

> **`ambre-crypto` intentionally absent.** The server only needs to verify Ed25519 signatures — direct `ed25519-dalek` suffices. Importing `ambre-crypto` would expose `DocumentKey`, `MasterSeed` in the server process without any legitimate need (violation of the least privilege principle).

> **Wire types imported from `ambre-protocol`.** `ambre-server` imports `WsClientMsg`, `WsServerMsg`, `UpdatePublicData`, `EphemeralPublicData` from `ambre-protocol` — no local redefinition.

**Invariants:**
- No plaintext data transits through the server (neither stored nor logged)
- ACLs are verified **before** any S3 read/write operation
- Every authenticated request validates the Redis session before touching PostgreSQL
- S3 presigned URLs are issued only after PostgreSQL ACL verification
- The Redis Lua script is atomic: clock validation + seq assignment in a single RTT
- A client's `signing_pubkey` is cached in memory in the Redis session — no PostgreSQL lookup per update
- No `unsafe` in application code

---

## 2. TECHNICAL DESIGN & STATE

### A. Data Modeling

#### PostgreSQL Schema

```sql
-- Users (OIDC profile + public crypto keys)
CREATE TABLE users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    oidc_provider   TEXT NOT NULL,
    oidc_subject    TEXT NOT NULL,
    email           TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    master_key_enc  BYTEA NOT NULL,          -- Master Seed encrypted by User Key (Argon2 client-side, Sync thread)
    master_key_salt BYTEA NOT NULL,          -- Argon2 salt to derive User Key client-side (Sync thread)
    signing_pubkey  BYTEA NOT NULL,          -- Ed25519 public key (signature verification)
    exchange_pubkey BYTEA NOT NULL,          -- X25519 public key (doc_key_enc encryption)
    recovery_hint   TEXT,                    -- first chars of Recovery Key (non-secret)
    org_id          UUID REFERENCES organizations(id),
    org_master_key_enc BYTEA,               -- Master Seed encrypted with Org Public Key (escrow)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (oidc_provider, oidc_subject)
);

-- Organizations (enterprise feature)
CREATE TABLE organizations (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL,
    org_pubkey       BYTEA NOT NULL,          -- X25519 organization public key
    shamir_threshold SMALLINT NOT NULL DEFAULT 3,
    shamir_total     SMALLINT NOT NULL DEFAULT 5,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE organization_members (
    org_id           UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id          UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role             TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    shamir_share_enc BYTEA,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);

CREATE TABLE org_recovery_log (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         UUID NOT NULL REFERENCES organizations(id),
    target_user_id UUID NOT NULL REFERENCES users(id),
    recovered_by   UUID NOT NULL REFERENCES users(id),
    reason         TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Documents (metadata only — content in S3)
-- doc_type: canonical French names, aligned with DocType enum in ambre-core
CREATE TABLE documents (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id   UUID NOT NULL REFERENCES users(id),
    doc_type   TEXT NOT NULL CHECK (doc_type IN ('prose', 'trame', 'toile', 'murmure')),
    name_enc   BYTEA,                        -- E2EE encrypted name
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Document ACLs
CREATE TABLE document_members (
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    doc_key_enc BYTEA NOT NULL,              -- Document Key encrypted with member's X25519 exchange_pubkey
    invited_by  UUID REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (document_id, user_id)
);

-- Snapshot index (blobs in S3)
CREATE TABLE snapshots (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id          UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    author_pubkey        BYTEA NOT NULL,
    parent_hash          BYTEA,              -- BLAKE2b hash of parent snapshot
    parent_snapshot_proof BYTEA,            -- Full BLAKE2b chain (from ambre-protocol)
    update_clocks        JSONB NOT NULL,     -- {"pub_key_hex": clock, ...}
    s3_key               TEXT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Delta index (blobs in S3)
CREATE TABLE updates (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    author_pubkey BYTEA NOT NULL,
    snapshot_id UUID NOT NULL REFERENCES snapshots(id),
    clock       BIGINT NOT NULL,
    seq         BIGINT NOT NULL,
    s3_key      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, author_pubkey, clock)
);
CREATE INDEX idx_updates_delta_sync ON updates (document_id, seq);

-- Media (content-addressable)
CREATE TABLE media (
    id          TEXT PRIMARY KEY,            -- BLAKE2b-256 hex of content
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    uploaded_by UUID NOT NULL REFERENCES users(id),
    mime_type   TEXT NOT NULL,
    size_bytes  BIGINT NOT NULL,
    s3_key      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Channels (Murmure messaging)
CREATE TABLE channels (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id   UUID NOT NULL REFERENCES users(id),
    name_enc   BYTEA,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE channel_members (
    channel_id      UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    channel_key_enc BYTEA NOT NULL,
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

-- "Last read message" cursors (navigation metadata — unencrypted)
CREATE TABLE channel_read_cursors (
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    last_seq   BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, channel_id)
);

-- Meetings (ephemeral — state in Redis, minimal Postgres archiving)
-- MVP: name in plaintext (meetings are ephemeral, not a critical secret)
-- Post-MVP: ephemeral MeetingKey to encrypt the name
CREATE TABLE meetings (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    creator_id UUID NOT NULL REFERENCES users(id),
    name       TEXT,                             -- MVP: plaintext. Post-MVP: name_enc BYTEA + MeetingKey
    status     TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'ended')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at   TIMESTAMPTZ
);
```

#### Redis/Valkey Structure

```
# Sessions (TTL 24h)
# signing_pubkey stored in session — avoids PostgreSQL lookup per update
session:{session_id}           → HASH { user_id, signing_pubkey_hex, created_at, last_seen }

# Global per-document sequence (SecSync seq)
doc:{doc_id}:seq               → STRING (monotonic INCR)

# Per-client per-document clocks (SecSync clock)
doc:{doc_id}:clocks            → HASH { pub_key_hex: clock_value }

# Delta buffer before S3 flush (drained by the global flusher)
doc:{doc_id}:pending           → LIST of serialized blobs { seq, s3_key, ciphertext_size }

# Compaction lock (SET NX, TTL 30s)
doc:{doc_id}:compact_lock      → STRING

# Messaging segment seal lock
channel:{channel_id}:seal      → STRING (SET NX, TTL 30s)

# Global per-channel sequence (SecSync seq — same pattern as documents)
channel:{channel_id}:seq       → STRING (monotonic INCR)

# Per-client per-channel clocks
channel:{channel_id}:clocks    → HASH { pub_key_hex: clock_value }

# Channel delta buffer before S3 flush
channel:{channel_id}:pending   → LIST of serialized blobs { seq, s3_key, ciphertext_size }

# Per-channel presence (encrypted blobs, same pattern as documents)
channel:{channel_id}:presence  → HASH { pub_key_hex: presence_enc_b64 }

# Active WebSocket sessions per channel
# Populated at SubscribeChannel, cleaned at UnsubscribeChannel or multiplexed WS disconnect
channel:{channel_id}:sessions  → HASH { session_id: "1" }

# Document broadcast PubSub
channel doc:{doc_id}           → PUB/SUB (postcard-serialized WsServerMsg)

# Channel broadcast PubSub (same pattern as documents)
channel chan:{channel_id}      → PUB/SUB (postcard-serialized WsServerMsg)

# Per-document presence
# IMPORTANT: presence data (cursor, selection) are encrypted blobs
# sent by the client in WsClientMsg::Ephemeral. The server stores the opaque blob.
# It never sees the cursor or the name in plaintext.
doc:{doc_id}:presence          → HASH { pub_key_hex: presence_enc_b64 }
                                  presence_enc = ciphertext of the Ephemeral payload
                                  MVP: periodic sweep (last_seen > 15s via auxiliary field)
                                  Valkey 8+: HEXPIRE 15s per field

# Active WebSocket sessions per document
# Populated at Subscribe, cleaned at Unsubscribe or multiplexed WS disconnect
doc:{doc_id}:sessions          → HASH { session_id: "1" }

# Reverse lookup user → open documents
user:{user_id}:open_docs       → SET { doc_id_1, doc_id_2 }

# Active meeting participants (non-sensitive metadata: joined_at, audio, video, screen)
meeting:{meeting_id}:participants → HASH { user_id: JSON{joined_at, audio, video, screen} }

# Meeting signaling PubSub
channel meeting:{meeting_id}   → PUB/SUB
```

> **Presence and Zero-Knowledge:** The value stored in `doc:{doc_id}:presence` is the opaque `Ephemeral.ciphertext` payload — the server never sees a participant's cursor, selection, or display name. Only `pub_key_hex` (the client's Ed25519 public key) is in plaintext, needed for the expiration sweep.

**Atomic Lua script — clock validation + seq assignment:**

```lua
-- KEYS[1] = doc:{id}:clocks, KEYS[2] = doc:{id}:seq
-- ARGV[1] = pub_key_hex, ARGV[2] = incoming clock
local current  = tonumber(redis.call('HGET', KEYS[1], ARGV[1])) or 0
local incoming = tonumber(ARGV[2])
if incoming ~= current + 1 then
    return redis.error_reply('CLOCK_MISMATCH:' .. current)
end
redis.call('HSET', KEYS[1], ARGV[1], incoming)
return redis.call('INCR', KEYS[2])
```

#### S3 Object Storage Hierarchy

```
{env}-ambre/
├── documents/
│   └── {doc_id}/
│       ├── snapshots/{snapshot_id}     -- encrypted full CRDT state
│       ├── deltas/{seq:010}            -- encrypted delta (zero-padded → lexicographic sort)
│       └── media/{content_hash}        -- encrypted blob (BLAKE2b-256, content-addressable)
│
└── channels/
    └── {channel_id}/
        ├── meta/
        │   ├── snapshots/{snapshot_id}
        │   └── deltas/{seq:010}
        ├── segments/
        │   └── {seg_id}/
        │       ├── snapshot            -- sealed segment: single immutable file
        │       └── deltas/{seq:010}
        └── media/{content_hash}
```

Conventions: `doc_id`, `channel_id`, `snapshot_id`, `seg_id` → UUIDv7. `seq` → zero-padded 10 digits. `content_hash` → BLAKE2b-256 hex (64 chars).

### B. Interface Contract (API)

#### Network Topology & Transport

```
Client (WASM)
  │
  ├── HTTP/3 (QUIC/UDP) ──► Caddy (TLS terminated) ──► Axum (internal HTTP/1.1)
  │       REST API (JSON) + WebSocket upgrade (binary postcard)
  │
  ├── Direct HTTPS ──► Object Storage S3 (presigned URLs — Caddy not involved)
  └── DTLS-SRTP (UDP) ──► str0m SFU (WebRTC media — separate service)
```

Axum never sees QUIC. Caddy absorbs TLS/QUIC termination and proxies in HTTP/1.1.

#### WebSocket Serialization

**Binary** WebSocket messages, serialized via `postcard`. Leading version byte:

```rust
// Types are imported from ambre-protocol -- not redefined here
use ambre_protocol::{WsClientMsg, WsServerMsg};

fn encode_ws_msg(msg: &WsServerMsg) -> Vec<u8> {
    let mut buf = vec![0x01]; // version byte
    postcard::to_extend(msg, &mut buf).expect("WsServerMsg serialization failed");
    buf
}

fn decode_ws_msg(data: &[u8]) -> Result<WsClientMsg, WsError> {
    match data.first() {
        Some(0x01) => Ok(postcard::from_bytes::<WsClientMsg>(&data[1..])?),
        Some(v)    => Err(WsError::UnsupportedVersion(*v)),
        None       => Err(WsError::EmptyFrame),
    }
}
```

The REST API uses **JSON** (Content-Type: application/json). Binary fields (keys, ciphertexts) are **base64url** encoded.

#### WebSocket Types — imported from `ambre-protocol`

```rust
// ambre-server imports these types, does not redefine them.
// Source of truth: ambre-protocol/src/ws.rs

// pub_key is always fixed [u8; 32] -- never Vec<u8>
// WsClientMsg::Ephemeral has a signature field: [u8; 64] to authenticate the sender
use ambre_protocol::{
    WsClientMsg,      // Subscribe, Unsubscribe, Update, Ephemeral (with signature), RequestMissing,
                      // SubscribeChannel, UnsubscribeChannel, ChannelUpdate, ChannelEphemeral
    WsServerMsg,      // Subscribed, Update (with seq), Ephemeral, MissingUpdates, SnapshotRequired,
                      // SnapshotAvailable, KeyRotationRequired, ChannelSubscribed, ChannelUpdate,
                      // ChannelEphemeral, Error
    UpdatePublicData, // document_id, pub_key: [u8; 32], snapshot_id, clock
    EphemeralPublicData, // document_id, pub_key: [u8; 32], counter
    SignalingClientMsg,
    SignalingServerMsg,
};
```

#### Axum Structure — AppState & Router

```rust
// rustis::client::Client is Clone (internal Arc) -- no pool needed
#[derive(Clone)]
pub struct AppState {
    pub db:           sqlx::PgPool,
    pub redis:        rustis::client::Client,  // RESP3 multiplexed, Clone-able
    pub s3:           aws_sdk_s3::Client,
    pub s3_bucket:    String,
    pub oidc_clients: Arc<HashMap<String, OidcClient>>,
    pub config:       Arc<Config>,
}

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/auth/{provider}/callback", post(auth::oidc_callback))
        .route("/auth/logout",              post(auth::logout))
        .route("/auth/me",                  get(auth::me))
        .route("/keys/master",              put(keys::store_master).get(keys::get_master))
        .route("/keys/master/rotate",       put(keys::rotate_master))
        .route("/users/search",             get(users::search))
        .route("/users/{id}/pubkeys",       get(users::pubkeys))
        .route("/documents",                post(docs::create).get(docs::list))
        .route("/documents/{id}",           get(docs::get).delete(docs::delete).patch(docs::update))
        .route("/documents/{id}/snapshot",  get(sync::doc_get_snapshot).post(sync::doc_create_snapshot))
        .route("/documents/{id}/snapshot/upload-url",   post(sync::doc_snapshot_upload_url))
        .route("/documents/{id}/snapshot/download-url", post(sync::doc_snapshot_download_url))
        .route("/documents/{id}/deltas",    get(sync::doc_get_deltas).post(sync::doc_push_deltas))
        .route("/documents/{id}/share",     post(share::doc_invite))
        .route("/documents/{id}/share/{uid}", delete(share::doc_revoke))
        .route("/documents/{id}/members",   get(share::doc_list_members))
        .route("/documents/{id}/members/{uid}", patch(share::doc_update_role))
        .route("/documents/{id}/rotate-key", post(share::doc_rotate_key))
        .route("/documents/{id}/media/upload-url",   post(media::doc_upload_url))
        .route("/documents/{id}/media/download-url", post(media::doc_download_url))
        .route("/documents/{id}/media",              post(media::doc_confirm_upload))
        .route("/channels",                 post(channels::create).get(channels::list))
        .route("/channels/{id}",            get(channels::get).delete(channels::delete).patch(channels::update))
        .route("/channels/{id}/members",    post(channels::invite).get(channels::list_members))
        .route("/channels/{id}/members/{uid}", delete(channels::revoke).patch(channels::update_role))
        .route("/channels/{id}/rotate-key", post(channels::rotate_key))
        .route("/channels/{id}/meta/snapshot", get(sync::chan_meta_get_snapshot).post(sync::chan_meta_create_snapshot))
        .route("/channels/{id}/meta/snapshot/upload-url",   post(sync::chan_meta_snapshot_upload_url))
        .route("/channels/{id}/meta/snapshot/download-url", post(sync::chan_meta_snapshot_download_url))
        .route("/channels/{id}/meta/deltas",   get(sync::chan_meta_get_deltas))
        .route("/channels/{id}/segments/{seg_id}/snapshot", get(sync::seg_get_snapshot).post(sync::seg_create_snapshot))
        .route("/channels/{id}/segments/{seg_id}/snapshot/upload-url",   post(sync::seg_snapshot_upload_url))
        .route("/channels/{id}/segments/{seg_id}/snapshot/download-url", post(sync::seg_snapshot_download_url))
        .route("/channels/{id}/segments/{seg_id}/deltas",   get(sync::seg_get_deltas).post(sync::seg_push_deltas))
        .route("/channels/{id}/segments/{seg_id}/seal",     post(sync::seg_seal))
        .route("/channels/{id}/read-cursor", put(channels::update_read_cursor))
        .route("/channels/unread",           get(channels::unread_counts))
        .route("/channels/{id}/media/upload-url",   post(media::chan_upload_url))
        .route("/channels/{id}/media/download-url", post(media::chan_download_url))
        .route("/channels/{id}/media",              post(media::chan_confirm_upload))
        .route("/meetings",                  post(meetings::create))
        .route("/meetings/{id}",             get(meetings::get).delete(meetings::end))
        .route("/meetings/{id}/join",        post(meetings::join))
        .route("/meetings/{id}/leave",       post(meetings::leave))
        .route("/organizations",             post(orgs::create))
        .route("/organizations/{id}",        get(orgs::get).patch(orgs::update).delete(orgs::delete))
        .route("/organizations/{id}/members",      post(orgs::add_member).get(orgs::list_members))
        .route("/organizations/{id}/members/{uid}", delete(orgs::remove_member).patch(orgs::update_member_role))
        .route("/organizations/{id}/recover/{uid}", post(orgs::recover_master_key))
        // Single multiplexed WebSocket (documents + channels on the same WS)
        .route("/ws",                          get(ws::upgrade))
        // Meetings: separate endpoint (SDP/ICE signaling, ephemeral lifecycle)
        .route("/ws/meetings/{meeting_id}",    get(ws::meeting_signaling_upgrade))
}
```

**Notes on key routes:**

- **`GET /api/ws`**: single multiplexed WebSocket per client session. Documents and channels share this connection. The handler maintains a `HashSet<Uuid>` of subscribed documents and channels. Dynamic Subscribe/Unsubscribe. Meetings keep a separate endpoint (`GET /api/ws/meetings/{id}`) because SDP/ICE signaling has an ephemeral lifecycle without a Subscribe pattern.
- **`GET /api/documents`**: returns for each document `{ id, doc_type, name_enc, role, doc_key_enc, updated_at }` — the `document_members` join includes `doc_key_enc` for the current user in the response. The Sync thread can decrypt all names in a single fetch, without N round-trips.
- **`POST /api/documents/{id}/media`** and **`POST /api/channels/{id}/media`**: confirmation routes after S3 upload. Body: `{ content_hash, mime_type, size_bytes }`. The server verifies ACL, inserts into the Postgres `media` table. Called by the Sync thread after successful S3 PUT.
- **`POST /api/.../media/upload-url`**: generates a presigned PUT URL (TTL 15 min) after ACL verification. No Postgres insertion — the INSERT is done by the confirmation route above.
- **`POST /api/documents/{id}/rotate-key`**: the `WsServerMsg::KeyRotationRequired` broadcast is emitted **at the end** of this route (after the Postgres transaction), not on `DELETE /share`. The new `doc_key_enc` values are in the database before other clients fetch them.

#### Blind Relay Flow — Update Sequence

```
Client (WASM Sync thread)
  │
  │  WsClientMsg::Update { ciphertext, nonce, public_data, signature }
  ▼
Axum WS Handler
  │
  ├── 1. Validates Redis session: session_id → { user_id, signing_pubkey_hex }
  │       (signing_pubkey cached in session — no PostgreSQL lookup)
  │
  ├── 2. Verifies document_id is in this connection's subscription set
  │       (in-memory HashSet<Uuid> per connection, populated by Subscribe)
  │
  ├── 3. Verifies PostgreSQL ACL: user_id ∈ document_members (role ∈ editor|owner)
  │       (result cacheable in Redis TTL 60s by {session_id, doc_id})
  │
  ├── 4. Verifies Ed25519 signature (direct ed25519-dalek):
  │       valid sign over (ciphertext || nonce || postcard(public_data))?
  │       public_data.pub_key == session's signing_pubkey_hex?
  │
  ├── 5. Redis Lua script (atomic):
  │       validates sequential clock → assigns global seq
  │
  ├── 6. Pushes to doc:{id}:pending (LIST) — { seq, ciphertext_bytes }
  │       flush to S3 by the global flusher (see below)
  │
  ├── 7. Publishes to Redis PubSub: channel doc:{doc_id}
  │       payload: postcard-serialized WsServerMsg::Update { seq, ... }
  │
  └── 8. Broadcasts WsServerMsg::Update { seq, ... } to all connected clients
          via local PubSub subscriptions to this Axum instance
```

#### Snapshot Flow — Upload/Download via S3 Presigned URLs

Snapshots do **not** transit via WebSocket (they can be multiple MB). The `Snapshot` type in `ambre-protocol` defines the S3 blob format (ciphertext + nonce + public_data + commitment_tag + signature, postcard-serialized).

```
Upload Snapshot (compaction initiator)
  Client (WASM Sync thread)
    │
    ├── 1. Encrypts snapshot → postcard-serialized blob (full Snapshot)
    │
    ├── 2. POST /api/documents/{id}/snapshot/upload-url
    │      → server verifies ACL (owner/editor), generates presigned PUT URL (TTL 15 min)
    │
    ├── 3. PUTs directly to S3 (opaque blob — Caddy not involved)
    │
    ├── 4. POST /api/documents/{id}/snapshot
    │      body: { snapshot_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key }
    │      → server INSERTs snapshots PostgreSQL
    │      → server purges prior deltas (seq <= clocks in update_clocks)
    │      → server Publishes doc:{doc_id} → WsServerMsg::SnapshotAvailable { snapshot_id }

Download Snapshot (other clients)
  Client (WASM Sync thread)
    │
    ├── 1. Receives WsServerMsg::SnapshotAvailable { snapshot_id }
    │
    ├── 2. POST /api/documents/{id}/snapshot/download-url
    │      body: { snapshot_id }
    │      → server verifies ACL, generates presigned GET URL (TTL 15 min)
    │
    ├── 3. GETs directly from S3 → postcard blob
    │
    └── 4. postcard::from_bytes → Snapshot → decryption → LogicEvent::RemoteSnapshot
```

#### Async S3 Flush — Global Model

The "one task per active document" model is rejected (see section 8). The flusher is a **single global Tokio task** per Axum instance:

```rust
/// Spawned once at startup.
/// Iterates over all documents with a non-empty pending LIST.
async fn global_s3_flusher(state: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;

        // SCAN over doc:*:pending keys to find documents to flush
        // Uses iterative rustis SCAN to avoid blocking Redis
        let mut cursor = 0u64;
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = state.redis
                .scan(cursor, Some("doc:*:pending"), Some(100))
                .await
                .unwrap_or((0, vec![]));

            // Flush in parallel (tokio::spawn per doc_id)
            let tasks: Vec<_> = keys.into_iter().map(|key| {
                let state = state.clone();
                tokio::spawn(async move { flush_doc_pending(&state, &key).await })
            }).collect();
            futures::future::join_all(tasks).await;

            cursor = next_cursor;
            if cursor == 0 { break; }
        }
    }
}

async fn flush_doc_pending(state: &AppState, pending_key: &str) {
    // LRANGE -> parallel S3 upload -> INSERT updates PostgreSQL -> LTRIM
    // Idempotent: if crash between LRANGE and LTRIM, next cycle re-flushes
    // S3 duplicates handled by unique key (zero-padded seq)
}
```

This model has O(1) Tokio tasks instead of O(active_documents). Redis SCAN is non-blocking. Flush latency stays ≤ 15s + SCAN duration.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

### Boundary Audit

| Data | What the server sees | Never seen |
|---|---|---|
| Document content | Opaque ciphertext | Plaintext text, numbers, images |
| Document names | `name_enc` (encrypted blob) | Plaintext name |
| Document keys | `doc_key_enc` (X25519 encrypted) | Plaintext Document Key |
| Master Seed | `master_key_enc` (encrypted client-side User Key Argon2) | Master Seed, passphrase |
| Pubkeys | `signing_pubkey` (Ed25519), `exchange_pubkey` (X25519) | Private keys |
| Presence | Opaque ciphertext (`presence_enc`) | Cursor, selection, plaintext name |
| Signatures | Ed25519 signature per message | — |
| Network metadata | Source IP, timestamps, blob sizes | — |
| Sessions | `session_id` → `user_id`, `signing_pubkey_hex` | Symmetric keys |

### Zero-Knowledge Invariants

1. Argon2 runs **client-side** (Sync WASM thread — sole consumer of `ambre-crypto`) — no passphrase reaches the server.
2. No symmetric key (User Key, Document Key, Meeting Key) is transmitted.
3. Tracing spans never include ciphertext content — only identifiers (doc_id, user_id, seq) are logged.
4. The compression version byte is inside the AEAD ciphertext — opaque to the server.
5. S3 presigned URLs have a TTL ≤ 15 min, generated after ACL check.
6. **Presence data is encrypted**: `doc:{doc_id}:presence` stores `presence_enc` (`Ephemeral.ciphertext` payload) — never the cursor or name in plaintext.

### Public Key Trust Model (TOFU)

`GET /api/users/search?email` and `GET /api/users/{id}/pubkeys` return `exchange_pubkey` + `signing_pubkey`. A compromised server could substitute these keys (server MITM attack).

**MVP Countermeasure — TOFU with out-of-band fingerprint:**
- On first invitation, the client displays the BLAKE2b-256 fingerprint (first 20 bytes, base32 encoded) of the fetched `exchange_pubkey`.
- The inviter can verify this fingerprint out-of-band with the invitee (SMS, in person).
- Accepted keys are stored in IndexedDB (`trusted_keys: HashMap<UserId, [u8; 32]>`). Any subsequent divergence triggers a "key changed" UI alert.
- **Documented and displayed limit in the UI:** without out-of-band verification, protection relies on trust in the server.
- **V2:** Key Transparency Log (append-only, Merkle-auditable).

### Authentication — OIDC

```
Client                  Axum Server              OIDC Provider
  │── GET /login ───────►│                              │
  │                      ├── generates state + nonce      │
  │                      │   (Redis session TTL 10min)   │
  │   redirect_url ◄─────┤                              │
  │── redirect ────────────────────────────────────────► │
  │         Authorization Code ◄────────────────────── │
  │── POST /api/auth/{p}/callback ──────────────────────►
  │                      ├── validates state             │
  │                      ├── exchanges code → tokens     │
  │                      ├── validates id_token JWT       │
  │                      ├── extracts sub + email         │
  │                      ├── upserts users PostgreSQL     │
  │                      ├── stores signing_pubkey in Redis session
  │                      ├── creates Redis session TTL 24h│
  │   Set-Cookie: session_id (HttpOnly, Secure, SameSite=Strict) ◄─┤
```

Providers: Google, GitHub, Microsoft, Apple.

### Session Management

```rust
pub struct AuthSession {
    pub session_id:       String,
    pub user_id:          Uuid,
    pub signing_pubkey:   [u8; 32], // Ed25519 -- from Redis, avoids PostgreSQL lookup per update
}

async fn auth_middleware(
    State(state): State<AppState>,
    cookie_jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let session_id = cookie_jar
        .get("session_id")
        .ok_or(AppError::Unauthorized)?
        .value()
        .to_string();

    let session: Option<SessionData> = state.redis
        .hgetall(format!("session:{session_id}"))
        .await?;

    let session = session.ok_or(AppError::Unauthorized)?;
    req.extensions_mut().insert(AuthSession::from(session));
    Ok(next.run(req).await)
}
```

### Document Key Rotation — Server Side

The full flow is specified in `ambre-spec-sync.md` §3. The initiator client calls both routes in sequence. Server responsibilities:

```
1. DELETE /api/documents/{id}/share/{uid}
   → DELETE FROM document_members WHERE document_id = $1 AND user_id = $2
   → NO broadcast here — the new keys are not yet in the database

2. POST /api/documents/{id}/rotate-key
   body: [{ user_id, doc_key_enc }] for all remaining members
   → Atomic PostgreSQL transaction:
     UPDATE document_members SET doc_key_enc = $new WHERE document_id = $id AND user_id = $uid
     for each (user_id, doc_key_enc) in the list
   → Verification: the user_ids in the list must all be in document_members
     (the revoked member was already deleted in step 1 — their key cannot be re-encrypted)
   → broadcast WsServerMsg::KeyRotationRequired { revoked_user_id } via PubSub
     (AT THIS POINT the new doc_key_enc values are in the database — no race condition.
      Other clients fetching GET /members will receive the new keys.)
```

### Network Security & Headers

```rust
// HSTS
.layer(SetResponseHeaderLayer::overriding(
    header::STRICT_TRANSPORT_SECURITY,
    HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
))
// Minimal CSP -- adjust based on used OIDC providers
.layer(SetResponseHeaderLayer::overriding(
    header::CONTENT_SECURITY_POLICY,
    HeaderValue::from_static("default-src 'self'"),
))
```

> ⚠️ **COOP/COEP Note (Model B — REQUIRED).** The client architecture has moved to **Model B**: 3 `wasm_thread` threads sharing a **linear memory** (`SharedArrayBuffer`) — see `ambre-spec-architecture.md §2.B`. `SharedArrayBuffer` requires the page to be **cross-origin isolated**, so the headers `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp` (or `credentialless`) **must** be served on the main HTML document (and the worker bootstrap). Consequence to document for self-hosting: any embedded cross-origin resource must carry appropriate `Cross-Origin-Resource-Policy` / CORS, otherwise it is blocked. *(The opposite was true in Model A shared-nothing, where these headers were not required — accepted cost of the move to copy-free typed `flume` transport.)*

TLS via **rustls** (Caddy as terminator). No OpenSSL in the Axum chain.

---

## 4. QUALITY & TESTABILITY

### Unit Tests

**Axum handlers**: `axum::test_helpers` with test `PgPool` (`sqlx::test`, rollbacked transactions) and mock `rustis::client::Client` or local Redis instance.

**Redis Lua script**: tested against local Valkey in CI. Cases: valid clock, invalid clock (CLOCK_MISMATCH), first update (current = 0), concurrency with two simultaneous clients.

**Postcard serialization**: round-trip on all `WsClientMsg` / `WsServerMsg` variants — imported from `ambre-protocol`.

**ACL**: each handler tested with owner, editor, viewer, non-member → HTTP code verification.

**Ed25519 signature verification**: update with valid signature → accepted. Invalid signature → 422. `pub_key` not matching session's `signing_pubkey` → 422.

### Prop-Testing / Fuzzing

`cargo-fuzz` on:
- `decode_ws_msg`: any arbitrary binary input → `Err`, never panic.
- Request parameter parsing (`after_seq`, UUIDs): no panic on malformed input.

`proptest`: property "seq is strictly increasing" regardless of clock arrival order on the Lua script.

### Environment

```yaml
# docker-compose.yml (local dev)
services:
  postgres:
    image: postgres:16-alpine
    environment: { POSTGRES_DB: ambre_dev, POSTGRES_PASSWORD: dev }
  valkey:
    image: valkey/valkey:8-alpine   # v8+ for per-field HEXPIRE
  minio:
    image: minio/minio
    command: server /data --console-address ":9001"
```

CI: `cargo test` with `DATABASE_URL` pointing to test PostgreSQL. `#[cfg(feature = "test-helpers")]` exposes helpers to create users/documents without going through OIDC.

### Benchmarks

| Endpoint | Target (p99) | Tool |
|---|---|---|
| WS Update (receive + validate + broadcast) | < 5ms | criterion + tokio-test |
| Redis Lua script (clock validation) | < 1ms | Direct Redis benchmark |
| S3 presigned URL | < 50ms | integration |
| GET deltas (100 entries) | < 20ms | k6 |

---

## 5. PERFORMANCE & RESOURCES

| Operation | Target | Potential bottleneck |
|---|---|---|
| WS Update receive + broadcast | < 5ms p99 | Lua Redis + PubSub |
| S3 presigned URL generation | < 50ms | S3 SDK (network I/O) |
| Document cold start (snapshot + N deltas) | < 500ms | S3 + Postgres read |
| OIDC auth callback | < 200ms | Provider token exchange |
| Global S3 flush batch (15s cycle) | < 2s for 1000 deltas | Parallel S3 upload |
| Presence sweep MVP (30s) | < 10ms | Redis SCAN |

**Horizontal scalability:** the server is stateless at the application level. State is in Redis (PubSub, sessions) and PostgreSQL. Multiple Axum instances coexist behind Caddy. Redis PubSub ensures WebSocket broadcasts reach clients on any instance.

**Memory:** no in-process blob cache. Buffer `LIST doc:{id}:pending` in Redis. Average delta ~1 KB. 15s buffer on an active document ≈ 50 deltas × 1 KB = ~50 KB per open document.

---

## 6. ERROR HANDLING

```rust
use thiserror::Error;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden: insufficient role")]
    Forbidden,
    #[error("Session expired")]
    SessionExpired,
    #[error("Not found: {resource}")]
    NotFound { resource: &'static str },
    #[error("Conflict: {detail}")]
    Conflict { detail: String },
    #[error("Clock mismatch: expected {expected}, got {got}")]
    ClockMismatch { expected: u64, got: u64 },
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("Snapshot required: too many pending deltas")]
    SnapshotRequired,
    #[error("Compaction lock held")]
    CompactionLockHeld,
    #[error("Unknown OIDC provider: {0}")]
    UnknownProvider(String),
    // Infrastructure errors -- 500 to the client, detail in logs
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("S3 error: {0}")]
    S3(String),
    #[error("Postcard decode error: {0}")]
    Decode(#[from] postcard::Error),
    #[error("OIDC error: {0}")]
    Oidc(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AppError::Unauthorized | AppError::SessionExpired
                => (StatusCode::UNAUTHORIZED,          "UNAUTHORIZED"),
            AppError::Forbidden
                => (StatusCode::FORBIDDEN,             "FORBIDDEN"),
            AppError::NotFound { .. }
                => (StatusCode::NOT_FOUND,             "NOT_FOUND"),
            AppError::Conflict { .. }
                => (StatusCode::CONFLICT,              "CONFLICT"),
            AppError::ClockMismatch { .. }
                => (StatusCode::CONFLICT,              "CLOCK_MISMATCH"),
            AppError::InvalidSignature
                => (StatusCode::UNPROCESSABLE_ENTITY,  "INVALID_SIGNATURE"),
            AppError::SnapshotRequired
                => (StatusCode::CONFLICT,              "SNAPSHOT_REQUIRED"),
            AppError::CompactionLockHeld
                => (StatusCode::CONFLICT,              "COMPACTION_LOCK_HELD"),
            AppError::UnknownProvider(_)
                => (StatusCode::BAD_REQUEST,           "UNKNOWN_PROVIDER"),
            _   => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        tracing::error!(error = %self, code = code);

        // Infra errors return a generic message -- never a stack trace
        let message = match &self {
            AppError::Database(_) | AppError::Redis(_) | AppError::S3(_)
                => "Internal server error".to_string(),
            other => other.to_string(),
        };

        (status, axum::Json(serde_json::json!({
            "error": code,
            "message": message,
        }))).into_response()
    }
}
```

**WebSocket errors**: sent via `WsServerMsg::Error { code, message }` before close, postcard-serialized.

---

## 7. REAL-TIME NOTIFICATIONS

### PubSub Architecture

**Multiplexed WebSocket:** Each WebSocket connection (`GET /api/ws`) maintains a `HashSet<Uuid>` of subscribed documents and a `HashSet<Uuid>` of subscribed channels. Redis PubSub subscribe/unsubscribe is dynamic: the server subscribes to the Redis channel `doc:{doc_id}` or `chan:{channel_id}` on `Subscribe`/`SubscribeChannel`, and unsubscribes on `Unsubscribe`/`UnsubscribeChannel` or disconnect. Meetings keep a separate WebSocket endpoint (`GET /api/ws/meetings/{id}`) because their ephemeral lifecycle (SDP/ICE signaling) does not follow the Subscribe/Unsubscribe pattern.

```
Axum instance A          Redis PubSub          Axum instance B
  │ Alice connected           │                   Bob connected │
  │ (multiplexed WS)           │                   (multiplexed WS) │
  ├── Subscribe doc:{id} ────►│◄─── Subscribe doc:{id} ────────┤
  │                           │                                 │
Alice sends Update            │                                 │
  ├── Publish doc:{id} ───────►│─── Message ───────────────────►│
  │                           │                   Broadcast → Bob┤
  │◄── (self) ────────────────┤
  ├── Broadcast → Alice
```

### Event Types

| Event | Redis Channel | Recipients |
|---|---|---|
| `Update` | `doc:{id}` | All connected members except sender |
| `Ephemeral` (encrypted presence) | `doc:{id}` | Same |
| `SnapshotRequired` | `doc:{id}` | All connected members |
| `SnapshotAvailable` | `doc:{id}` | All connected members except sender |
| `KeyRotationRequired` | `doc:{id}` | All except revoked member |
| `ParticipantJoined/Left` | `meeting:{id}` | All participants |
| Signaling (Offer/Answer/ICE) | `meeting:{id}` | Targeted recipient only |

### Heartbeat and Presence (MVP)

The client sends a `WsClientMsg::Ephemeral` (encrypted payload) every ~10s. The server:
1. `HSET doc:{id}:presence {pub_key_hex} {presence_enc_b64}` in Redis
2. Forwards to all other clients via PubSub (the blob is opaque — pure relay)

Sweep every 30s: purge entries whose `last_seen > 15s` (auxiliary field `doc:{id}:presence_ts:{pub_key_hex}` → TTL 15s).

**Valkey 8+ migration:** `HEXPIRE doc:{id}:presence {pub_key_hex} 15` on each heartbeat. No more sweep timer.

### Offline Notifications

No push notifications in MVP. The client queries `GET /api/channels/unread` on reconnect. `unread_count = max_seq_in_channel - last_seq_user` (`channel_read_cursors` table).

---

## 8. REJECTED ARCHITECTURAL DECISIONS

| Technology / Approach | Reason |
|---|---|
| **`ambre-crypto` as a dependency of `ambre-server`** | Violates least privilege: exposes `DocumentKey`, `MasterSeed`, Argon2 in the server process. The only crypto operation needed is Ed25519 verification — direct `ed25519-dalek` suffices. |
| **Redefining wire types in `ambre-server`** | `ambre-protocol` is the source of truth. Redefining locally creates guaranteed divergence (e.g., `pub_key: Vec<u8>` vs `[u8; 32]` observed in the original). |
| **`deadpool_redis` / `redis-rs`** | Replaced by `rustis` (native RESP3 client, multiplexed, Clone-able `Client` without pool). |
| **`doc_type` with English names** | `spreadsheet`, `document`, `whiteboard` diverge from application names. Canonical French names: `prose`, `trame`, `toile`, `murmure`. |
| **Plaintext Redis presence** | `doc:{id}:presence` stored `name`, `color`, `cursor` as plaintext JSON — ZK violation. Replaced by `presence_enc` (opaque Ephemeral ciphertext). |
| **S3 flush: one Tokio task per active document** | Not scalable to >1000 active documents (1000 dormant tasks). Replaced by a single global flusher with Redis SCAN. |
| **PostgreSQL lookup of `signing_pubkey` per update** | Adds a PostgreSQL query on the hot path (every WS update). `signing_pubkey` is cached in the Redis session at login. |
| **Axum with native QUIC (quinn)** | Caddy as HTTP/3 terminator is lower operational risk. Axum only sees internal HTTP/1.1. |
| **SSE instead of WebSockets** | Unidirectional (server → client). The client must send real-time updates. |
| **One WebSocket per document/channel** | Connection waste (5 WS for a Toile with 4 embedded documents). Browser limits (~6 connections per origin). Messages already carry `document_id` in `publicData` — the server demultiplexes. Single multiplexed WS per session with Subscribe/Unsubscribe, single endpoint `GET /api/ws`. Meetings keep a separate endpoint (ephemeral signaling). |
| **gRPC / Protobuf** | No native WASM browser support without grpc-web proxy. Postcard + WebSocket is simpler. |
| **External message broker (NATS, Kafka)** | Over-engineering for MVP. Redis PubSub suffices. NATS JetStream migration possible in V2. |
| **Stateless JWT for sessions** | Not immediately revocable. Redis sessions allow instant revocation. |
| **ORM (Diesel, SeaORM)** | Diesel blocks native async. SeaORM obscures critical queries. sqlx with compile-time check is the right abstraction level. |
| **S3 blobs proxied by the server** | The server must not see the content. Presigned URLs: direct upload/download client → S3. |
| **`WsClientMsg::Ephemeral` without signature** | Without a signature, any client could forge a presence message on behalf of another. `signature: [u8; 64]` is mandatory (defined in `ambre-protocol`). |

---

## 9. INTEGRATION PLAN (ROADMAP)

> Detailed backlog of this spec; cross-cutting scheduling (milestones) lives in `specs/ambre-roadmap.md`.

### SRV P0 — Core (Functional Blind Relay)

- [x] Full PostgreSQL schema with sqlx migrations (`doc_type` in French names)
- [ ] OIDC auth (Google first)
- [x] Redis sessions (`rustis`) + auth middleware + `signing_pubkey` cache in session
- [x] Documents CRUD + basic ACL
- [x] `GET /api/documents` with inline `doc_key_enc` (`document_members` join)
- [x] Single multiplexed WebSocket (`GET /api/ws`): per-document Subscribe/Unsubscribe, Update + Ephemeral + MissingUpdates + SnapshotRequired + SnapshotAvailable. HashSet<Uuid> of subscriptions per connection. Dynamic Redis PubSub.
- [x] Redis Lua script (clock + seq)
- [ ] Global S3 flusher (single Tokio task, Redis SCAN) — *flusher loop + SCAN done; S3 upload not wired*
- [x] S3 presigned URLs (snapshots + media)
- [x] Media routes: upload-url + download-url + confirmation POST
- [x] Typed error handling (`thiserror` → HTTP)
- [x] Structured logging (`tracing` → JSON → Promtail → Loki)

### SRV P1 — Channels + Meetings

- [ ] Channels CRUD + members + ACL
- [ ] Channel Redis keys (seq, clocks, pending, presence, sessions, PubSub)
- [x] SubscribeChannel/UnsubscribeChannel on the existing multiplexed WS (same SecSync protocol, same connection)
- [ ] Channel presigned URLs (meta snapshots + segment snapshots + media + confirmation POST)
- [ ] Segments: sealing, history
- [ ] Unread tracking (PostgreSQL cursors)
- [ ] Meetings: creation, join/leave, WebSocket signaling (MVP: plaintext name)
- [x] Document Key rotation (server flow: DELETE share + POST rotate-key — broadcast on rotate-key)
- [x] `KeyRotationRequired` notifications via PubSub (emitted on POST /rotate-key, not DELETE /share)

### SRV P2 — Hardening & Organizations

- [ ] Per-user/IP rate limiting (tower middleware)
- [ ] CSRF protection (Double Submit Cookie on POST/PATCH/DELETE)
- [ ] Organizations feature + Shamir's Secret Sharing (crate `sharks`)
- [ ] Route `POST /api/organizations/{id}/recover/{uid}`
- [ ] Server-side compaction (cron: purge old snapshots > 10 per document)
- [ ] S3 lifecycle policies (messaging segments > 12 months)
- [ ] Load tests (k6: 1000 simultaneous users, 100 active documents)
- [ ] Valkey 8+ migration (per-field HEXPIRE for presence)
