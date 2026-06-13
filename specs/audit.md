# AMBRE — CODE AUDIT

> Full codebase audit across 7 crates, ~15 000 lines. Findings grouped by severity.
> Date: 2026-06-07

---

## CRITICAL (3)

### C1 — Ciphertext logged in S3 flusher error paths

**Crate:** `ambre-server`  
**File:** `crates/ambre-server/src/s3.rs:177,185`

```rust
tracing::error!(blob = %blob_str, error = %e, "flush: json parse failed");
tracing::error!(blob = %blob_str, "flush: missing s3_key");
```

`blob_str` contains `ciphertext_b64` — the full encrypted delta content base64-encoded. Logging ciphertext increases exposure of encrypted material, fills logs with large base64 strings, and could aid cryptanalytic attacks if key rotation ever exposes old keys.

**Fix:** Redact `blob_str` fields before logging; keep only `seq`, `document_id`, `s3_key`.

---

### C2 — Missing Redis PubSub subscriber (cross-instance sync broken)

**Crate:** `ambre-server`  
**Files:** `crates/ambre-server/src/redis_ops.rs:193-204`, `crates/ambre-server/src/ws/handler.rs`

`doc_broadcast()` PUBLISHes messages to Redis PubSub, but there is **no corresponding SUBSCRIBE loop** in the crate. In a multi-instance deployment, Instance 1 broadcasts an update, but Instance 2 (with WebSocket connections subscribed to the same document) never receives it. Cross-instance real-time sync is non-functional until a subscriber is wired.

**Fix:** Add a Redis PubSub subscriber task that dispatches incoming messages to the local WebSocket connections.

---

### C3 — Timing side-channel in HMAC commitment verification

**Crate:** `ambre-crypto`  
**File:** `crates/ambre-crypto/src/commitment.rs:46-48`

```rust
mac(key, nonce, ciphertext, aad)
    .verify_slice(tag)
    .map_err(|_| CryptoError::CommitmentMismatch)
```

`Cargo.lock` resolves `hmac` to **0.12.1**. `verify_slice` in versions ≤ 0.12.1 uses a non-constant-time byte comparison. An attacker who can measure the timing of `verify_commitment` can brute-force the HMAC tag byte-by-byte.

**Fix:** Upgrade `hmac` to 0.12.2+ or 0.13.x (already present as a transitive dep in Cargo.lock).

---

## HIGH (7)

### H1 — WebSocket handler silently drops DB/Redis errors (data loss)

**Crate:** `ambre-server`  
**File:** `crates/ambre-server/src/ws/handler.rs:204-215`

```rust
let _ = db::insert_update(...).await;
let _ = redis_ops::append_pending_delta(...).await;
let _ = redis_ops::doc_broadcast(...).await;
```

The seq has already been assigned by the atomic Lua `INCR` (irreversible in Redis). If `insert_update` (Postgres) fails, no metadata row exists for this seq — the S3 delta exists but is unreferenced. If `append_pending_delta` (Redis list) fails, the delta never gets flushed to S3. The client receives no error because the seq was already broadcast.

**Fix:** At minimum, log errors at `ERROR` level. Ideally, implement a retry or a dead-letter queue for failed writes.

---

### H2 — Document metadata leaked to non-members

**Crate:** `ambre-server`  
**File:** `crates/ambre-server/src/routes/documents.rs:153-170`

If `get_document_member` returns `None` (caller is not a member), the handler still returns the document's `id`, `owner_id`, `doc_type`, and `name_enc`. While `id` is UUIDv4 (unguessable), any leaked document ID exposes structure to unauthorized users.

**Fix:** Return 403 or 404 when `mem` is `None`.

---

### H3 — Silent fallback to random DocumentKey on ECDH failure

**Crate:** `ambre-sync`  
**File:** `crates/ambre-sync/src/lib.rs:1308-1311`

```rust
match &exchange_key {
    Some(ek) => Self::unwrap_document_key(ek, &resp.members)
        .unwrap_or_else(|_| DocumentKey::generate()),
    None => DocumentKey::generate(),
}
```

If ECDH key unwrapping fails (wrong key, corrupted server data), the code falls back to a fresh random key. The document will appear to open successfully but every subsequent decrypt attempt will fail with confusing errors.

**Fix:** Propagate the error to the Logic thread via `AuthError` instead of generating a random key.

---

### H4 — Passphrases not zeroized in stub handlers

**Crate:** `ambre-sync`  
**File:** `crates/ambre-sync/src/lib.rs:784-796`

```rust
fn handle_change_passphrase(&mut self, current_passphrase: Vec<u8>, new_passphrase: Vec<u8>) {
    tracing::info!("Sync: ChangePassphrase");
    let _ = (current_passphrase, new_passphrase); // dropped without zeroization
}
```

`let _ = (...)` binds and immediately drops the tuple. `Vec<u8>` does not zeroize on drop. The passphrase bytes remain recoverable in the WASM heap.

**Same issue:** `handle_recover_account` (`recovery_words: String`, `new_passphrase: Vec<u8>`).

**Fix:** Wrap in `Zeroizing<Vec<u8>>` or call `.zeroize()` explicitly before drop.

---

### H5 — Passphrases and recovery words travel plaintext across IPC

**Crate:** `ambre-bus`  
**File:** `crates/ambre-bus/src/rest.rs:129-146,144`

`RestCommand::Unlock { passphrase: Vec<u8> }`, `SetupAccount { passphrase: Vec<u8> }`, `ChangePassphrase { current_passphrase, new_passphrase }`, `RecoverAccount { recovery_words: String }` all travel through the Logic thread as `LogicCommand::Rest { cmd }` before reaching Sync. Neither the UI-side nor the Logic-thread copies are zeroized. The recovery words (BIP39 mnemonic) are equivalent to the master key and travel as a plain `String`.

**Fix:** Zeroize passphrase copies after they are sent; consider using `Zeroizing<Vec<u8>>` in the IPC types.

---

### H6 — Remote CRDT edits silently dropped (`apply_remote` is a stub)

**Crate:** `ambre-trame`  
**File:** `crates/ambre-trame/src/logic.rs:236-238`

```rust
pub fn apply_remote(&mut self, _payload: &[u8]) -> Vec<TrameEvent> {
    // TODO: import the Loro delta and recompute the affected viewport.
    vec![]
}
```

All remote edits from collaborators are silently discarded. Multi-user collaborative editing is non-functional. The TODO confirms this is a known gap.

**Fix:** Implement `loro::LoroDoc::import` on the remote delta and recompute the affected viewport.

---

### H7 — Stale formula cache on plain-text conversion

**Crate:** `ambre-trame`  
**Files:** `crates/ambre-trame/src/formula/engine.rs:52-62`, `crates/ambre-trame/src/logic.rs:116-121`

When a user changes a cell from `=42` to plain text `hello`, `on_cell_changed` recompiles but discards the recompile result (`let _ = ...`). Then `compute_all_dirty` re-inserts the **old** formula result from the cache. The cell displays `42` instead of `hello`. `cell_text` in `logic.rs` returns the cached formula result without checking `is_formula_cell()`.

**Fix:** When a cell is no longer a formula, remove its cache entry and fall through to the raw CRDT text.

---

## MEDIUM (12 selected — full list of ~40 available in detailed sections below)

### M1 — `matchMedia("(resolution)")` listener is non-functional

**Crate:** `ambre-canvas`  
**File:** `crates/ambre-canvas/src/host.rs:709-714`

```rust
web_sys::window()
    .and_then(|w| w.match_media("(resolution)").ok())
    .flatten()
```

The CSS media feature `(resolution)` without a value is a "supports" test — it is always true and never fires on DPI changes. DPI-change detection via matchMedia does not work. If the window moves to a different-DPI monitor without resizing, the canvas renders at the wrong resolution.

**Fix:** Use `(min-resolution: ...)` or `(resolution: ...)` with a proper value, or watch `devicePixelRatio` via `ResizeObserver` + manual DPR check.

---

### M2 — No GPU context loss / device lost handling

**Crate:** `ambre-canvas`  
**Files:** entire crate

There is zero handling for `webglcontextlost`, `webglcontextrestored`, `wgpu::Device::on_uncaptured_error`, or GPU OOM. If the browser's GPU process crashes or VRAM is exhausted on mobile, the canvas stops rendering permanently with no recovery mechanism.

**Fix:** Register `webglcontextlost`/`webglcontextrestored` event handlers and implement device re-creation.

---

### M3 — GlyphAtlas full-clear on overflow instead of LRU eviction

**Crate:** `ambre-canvas`  
**File:** `crates/ambre-canvas/src/render/mod.rs:248-253`

When the 2048×2048 atlas fills, `needs_clear = true` is set. On the next frame, **all** cached glyphs are evicted. The next frame must re-rasterize and re-upload every visible glyph via Swash. This causes a visible frame stutter.

**Fix:** Implement LRU or LFU eviction on individual glyph entries instead of a full clear.

---

### M4 — TextCache all-or-nothing eviction causes frame stutter

**Crate:** `ambre-canvas`  
**File:** `crates/ambre-canvas/src/text_cache.rs:72-76`

When the cache exceeds 5 000 entries, `self.layouts.clear()` clears everything. The next frame must call `Parley::shape()` for every visible text element (potentially hundreds). This creates a single-frame stutter every ~5 001 cache insertions.

**Fix:** Implement weighted LRU eviction (evict the oldest/least-used entries, not all).

---

### M5 — Stroke rect corner double-blend for translucent colors

**Crate:** `ambre-canvas`  
**File:** `crates/ambre-canvas/src/render/mod.rs:1169-1188`

`stroke_rect` draws 4 overlapping edge quads (top, bottom, left, right). At each of the 4 corners, the horizontal and vertical edge quads overlap. For semi-transparent strokes, the double-blend produces visibly incorrect results (corners appear brighter/darker than edges).

**Fix:** Use a single mitered quad strip, or clip corner overlaps with scissor rects.

---

### M6 — Rate limiter fail-open on Redis outage + fixed-window burst

**Crate:** `ambre-server`  
**File:** `crates/ambre-server/src/middleware/rate_limit.rs:147-168,217-223`

- **Fail-open:** Any Redis error disables rate limiting entirely. An attacker who can cause Redis disruption can send unlimited requests.
- **Fixed-window burst:** An attacker can send `max_req` at `t=59.9s` and another `max_req` at `t=60.1s`, achieving 2× the intended limit in ~200ms.
- **Race condition:** If the server crashes between `INCR` and `EXPIRE`, the counter key exists with no TTL and permanently consumes a request slot.

**Fix:** Use sliding-window counters (sorted sets); use `SET key 1 EX ttl_secs NX` for atomic TTL assignment; implement fail-closed with a degraded mode.

---

### M7 — OIDC discovery failure silently ignored at startup

**Crate:** `ambre-server`  
**File:** `crates/ambre-server/src/main.rs:79-86`

```rust
let oidc_clients = oidc::init_oidc_clients(...)
    .await
    .ok()                // error silently swallowed
    .unwrap_or_default(); // empty OIDC map
```

If Google's OIDC discovery endpoint is unavailable at startup, the server starts with zero OIDC providers. All login attempts return `UnknownProvider`. The outer `.ok()` discards the error entirely.

**Fix:** Log the error at `ERROR` level. Consider a startup health check that fails fast if OIDC is misconfigured.

---

### M8 — `ambre-logic` persistence.rs is dead code (284 lines never compiled)

**Crate:** `ambre-logic`  
**Files:** `crates/ambre-logic/src/lib.rs`, `crates/ambre-logic/src/persistence.rs`

There is no `mod persistence;` in `lib.rs`. The entire `persistence.rs` file (284 lines, `PersistenceStore`, IndexedDB scaffolding) is never compiled. Additionally, 5 dependencies in `Cargo.toml` are unused by compiled code: `futures-util`, `wasm-bindgen`, `wasm-bindgen-futures`, `js-sys`, `web-sys` (10 features).

**Fix:** Either wire `mod persistence;` in `lib.rs` (if offline durability is ready) or remove the dead file and its dependencies until ready.

---

### M9 — No HTTP timeouts in Sync thread

**Crate:** `ambre-sync`  
**File:** `crates/ambre-sync/src/http.rs`

None of `get_json`, `post_json`, `delete`, `fetch_bytes`, `put_bytes`, `put_json` set any timeout. `gloo-net` requests without explicit timeout can hang indefinitely (up to ~300s in browsers). In a WASM Worker, this would block the Sync thread's `select!` loop until the fetch times out at the browser level.

**Fix:** Add a default timeout (e.g., 30s) to all HTTP requests via `AbortController` or `gloo-net`'s timeout mechanism.

---

### M10 — `WsError` does not trigger reconnection

**Crate:** `ambre-sync`  
**File:** `crates/ambre-sync/src/lib.rs:931-933`

```rust
SyncEvent::WsError => {
    tracing::error!("Sync: WebSocket error");
}
```

Compare with `WsDisconnected` which clears state AND triggers reconnection. A WebSocket error without a close event leaves the socket in a zombie state with no recovery.

**Fix:** Trigger the same reconnection logic for `WsError` as for `WsDisconnected`.

---

### M11 — `UpdateReadCursor` has no `RestEvent` response (breaks 1:1 pattern)

**Crate:** `ambre-bus`  
**File:** `crates/ambre-bus/src/rest.rs:246-249`

Every other `RestCommand` variant has a corresponding `RestEvent` variant. `UpdateReadCursor` does not — the comment says "Acknowledged implicitly." The UI thread cannot confirm delivery or retry on failure.

**Fix:** Add a `ReadCursorUpdated` variant to `RestEvent`, or document the fire-and-forget semantics explicitly.

---

### M12 — `AuthError`/`ApiError` lack `RequestId` for correlation

**Crate:** `ambre-bus`, `ambre-sync`  
**Files:** `crates/ambre-bus/src/event.rs:110-120`, `crates/ambre-bus/src/ui.rs:111-123`, `crates/ambre-sync/src/lib.rs:977-983`

The `HttpError` handler in Sync discards the `id` (`_id`) and sends a generic `ApiError` with hardcoded `code: 500` and `request: "rest"`. If two concurrent HTTP requests are in flight, the caller cannot distinguish which one failed.

**Fix:** Add `Option<RequestId>` to `AuthError`/`ApiError`; preserve the `RequestId` from `HttpError` through the event bus.

---

## OTHER NOTABLE FINDINGS

### ambre-canvas

- **`caps.formats[0]` direct indexing** (`host.rs:212`): panics if `caps.formats` is empty (unlikely but not impossible on headless backends).
- **`matchMedia` listener registration failures silently discarded** (`host.rs:713`): `let _ = mql.add_event_listener_with_callback(...)` — failure is invisible.
- **Surface resize race** (`host.rs:374-375`): correctness depends on unwritten contract that `AppView::resize()` must flag a full repaint — no runtime assertion.
- **WGSL divide-by-zero** (`solid.wgsl:33`, `glyph.wgsl:33`): division by `u.viewport.x`/`u.viewport.y` without epsilon guard (not reachable in practice since dimensions are clamped to >=1).

### ambre-trame

- **`flush_delta` swallows Loro errors** (`logic.rs:227`): `.ok()` converts any export error to `None` — delta permanently lost.
- **Scroll accumulator lost at grid edges** (`ui/view.rs:765-781`): accumulator is decremented before checking `scroll_by` success — momentum silently discarded.
- **4 independent implementations of `col_to_alpha`** (`logic.rs`, `formula/types.rs`, `ui/view.rs`, `ui/renderer.rs`): identical algorithm copied 4 times.
- **5 copies of glyph-drawing loop** (`ui/renderer.rs`): same 6-line `for line in layout.lines()` pattern duplicated across 5 functions.
- **Range registration is O(range_size)** (`formula/dag.rs:126-135`): `=SUM(A1:ZZ100000)` would iterate 100M times.
- **`Pow` no infinity/NaN check** (`formula/vm.rs:106-113`): unlike `binop` which returns `#NUM!`, `Pow` returns raw `NaN`/`inf`.
- **`LEFT`/`RIGHT` negative length wraps via `as usize`** (`formula/functions.rs:279,295,319`): `-1.0 as usize` wraps to `usize::MAX`, returning the entire string.
- **`as u32` on negative floats is UB** (`ui/grid.rs:349-350`, `ui/renderer.rs:128,136`): guarded in current code paths but fragile if called from new contexts.
- **Missing keyboard shortcuts**: no Ctrl+A, Shift+Arrow, PageUp/PageDn, Ctrl+Home/End, Ctrl+Arrow, copy/paste.
- **No `CompositionStart` handler** (`ui/view.rs`): some IMEs require this event to initialize.
- **Active cell border disappears at right edge** (`ui/renderer.rs:348-350`, `ui/grid.rs:368-369`): `cell_pixel_rect` returns `None` for partially-visible cells instead of a clipped rect.

### ambre-sync

- **`QUARANTINE_ALERT` threshold never tested** (`lib.rs`): no test verifies `RequestMissing` is triggered at 900 quarantined updates.
- **`open_snapshot` dead code** (`pipeline.rs:165`): only used in its own test.
- **Channel handlers are all empty stubs** (`lib.rs:561-593,1170-1176,2217-2226`): `ListChannels`, `CreateChannel`, `DeleteChannel`, etc. only emit `tracing::info!`. Channel data from the server is completely discarded.
- **Meeting handlers are empty stubs** (`lib.rs:584-593`): `CreateMeeting`, `JoinMeeting`, etc. are no-ops.
- **Ephemeral remote counters have unbounded growth** (`lib.rs:229`): no eviction policy for `HashMap<[u8; 32], u64>` per document.
- **WASM path has zero integration test coverage**: `run_wasm()`, the entire WebSocket callback wiring, `handle_sync_event`, and the `select!` loop are untested.
- **Error messages leak encryption scheme details** (`lib.rs:615-670`): `AuthError` messages reveal key lengths, algorithm names, and library error details.

### ambre-server

- **Latent panic in `get_org_shamir_shares`** (`db.rs:921-922`): `.unwrap()` on `try_get("user_id")` — not called today but would crash the server if wired.
- **`setup_account` creates unlimited accounts with hardcoded email** (`documents.rs:40-63`): `"temp@ambre.app"`, `"User"` — no rate limiting beyond global auth tier.
- **WebSocket session never re-validated** (`ws/handler.rs:51-59`): authenticated once at upgrade, never checked for expiry during long-lived connections.
- **Editors can rotate document keys** (`share.rs:73`): should probably be owner-only.
- **No server-side verification of S3 uploads** (`sync.rs:38-58`, `media.rs:61-73`): DB rows inserted referencing S3 keys that may not exist.
- **Clock hash unbounded growth** (`redis_ops.rs:17-25`): per-author clock entries in Redis hash never cleaned up.
- **Flusher re-uploads successfully-flushed entries on partial failure** (`s3.rs:227,233`): `break` skips `ltrim` — all entries remain in the pending list.
- **CSRF exemption overly broad** (`csrf.rs:40`): entire `/api/auth/` prefix exempted, not individual routes.
- **No Redis reconnection logic** (`main.rs:57-59`): if `rustis` internal reconnection fails, all Redis ops fail permanently.

### ambre-logic

- **Silent return on missing document** (`lib.rs:143,158`): `apply_remote` and `apply_ephemeral` silently return when the document is not found, unlike `handle_app_command` which logs a warning.
- **16 event/command arms untested** (`lib.rs:259-314`): many `LogicEvent` variants (RemoteUpdate, RemoteEphemeral, ConnectionStatus, etc.) have zero test coverage.

### ambre-bus

- **`ChannelSnapshotNeeded` has no `SendChannelSnapshot`** (`event.rs:92`, `command.rs:69-85`): the server can request a channel snapshot but the Logic thread has no IPC path to send one.
- **`OrganizationId` missing** (`role.rs:69-77`): `Role` supports orgs but no org identifier type exists in `ambre-core`.
- **Single `CHANNEL_CAPACITY = 1024` for all channels** (`app/lib.rs:55`): a single large `SendSnapshot` can be megabytes; 1024 of them would exhaust WASM memory with no per-message size check.

### ambre-crypto

- **Multiple intermediate keys not zeroized**: `derive_exchange_key` (`keys.rs:59-62`), ECDH shared secret (`ecdh.rs:51-54,63-64`), KEK derivation (`ecdh.rs:35-43`), `derive_user_key` return (`passphrase.rs:21-29`), BLAKE2b digest in `derive32` (`keys.rs:24-32`).
- **`DocumentKey` and `MasterSeed` derive `Clone`** (`keys.rs:36,72`): allows silent duplication of secret key material.
- **No length-prefix on envelope fields** (`signature.rs:14-21`): boundary between `ciphertext` and `aad` is implicit.
- **`Vec<u8>` ciphertext not zeroized** (`envelope.rs:25`): `Sealed.ciphertext` is not wiped on drop.
- **Argon2id parameters at low end of recommendations** (`passphrase.rs:16-18`): 64 MiB, 3 iterations, 1 lane — meets OWASP minimums but should be tuned upward.
- **AEAD encrypt panics on hypothetical buffer overflow** (`aead.rs:24`): `.expect()` instead of returning an error.

---

## POSITIVE FINDINGS

- **Zéro SQL injection** on the server — all queries use `sqlx::query*()` with `.bind()`.
- **Zéro `unsafe` block** in `ambre-crypto`, `ambre-canvas`, and `ambre-bus`.
- **Architectural separation enforced by the crate dependency graph** — each crate respects its boundaries (verified via `cargo tree`).
- **Encrypt-then-MAC-then-Sign** envelope ordering is correct (AEAD → HMAC → Ed25519).
- **Domain separation** present for all key derivations (`SIGN_DOMAIN` vs `EXCH_DOMAIN` vs `KEK_DOMAIN`).
- **`ZeroizeOnDrop`** on `MasterSeed` and `DocumentKey`.
- **Atomic Lua script** for clock+seq generation — no race condition.
- **Constant-time Ed25519 verification** via `ed25519-dalek`.
- **CSPRNG nonces** via `getrandom` (192-bit XChaCha20).
- **No postcard/serde leakage into `ambre-bus`** — the IPC crate has zero serialization dependencies.
