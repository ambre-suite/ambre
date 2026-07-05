# AMBRE — CODE AUDIT

> Full-codebase audit across 16 crates, ~51 000 lines of Rust. Findings grouped by severity.
> **Date:** 2026-07-05 (supersedes the 2026-06-07 pass on 7 crates / ~15 000 lines)
>
> **Scope.** `crates/*` (server, sync, crypto, protocol, bus, core, logic, trame,
> canvas, chrome, commands, draw, app; `murmure`/`prose`/`toile` are still stubs)
> plus `specs/**`. Cross-checked against the specifications; deviations are called out.
>
> **Caveat on dependency versions.** `Cargo.lock` was **not** part of the snapshot
> audited. Any finding whose severity depends on a resolved dependency version
> (Argon2, hmac, ed25519-dalek, ruzstd, aws-sdk-s3) is flagged accordingly and
> should be re-confirmed against the actual lockfile.

---

## STATUS OF PREVIOUS FINDINGS

Verified against the current tree. IDs are preserved for traceability.

| ID | Title | Status (2026-07-05) |
| :-- | :-- | :-- |
| C1 | Ciphertext logged in S3 flusher error paths | **OPEN** — `s3.rs:215,223` still log `blob_str` (contains `ciphertext_b64`). |
| C2 | Missing Redis PubSub subscriber (cross-instance sync broken) | **RESOLVED** — `ws/pubsub_hub.rs` (`PubSubHub::spawn`, subscribe/unsubscribe fan-out) is wired in `main.rs`. |
| C3 | Timing side-channel in HMAC commitment verification | **FALSE POSITIVE** — see "Corrections to the previous audit" below. `Mac::verify_slice` is constant-time. |
| H1 | WS handler silently drops DB/Redis errors | **PARTIAL** — `insert_update` now logs at ERROR; `append_pending_delta` and `doc_broadcast` are still `let _ =`. |
| H2 | Document metadata leaked to non-members | **OPEN** — `documents.rs` `get_document` still returns `id`/`owner_id`/`doc_type`/`name_enc` when `mem == None`. |
| H3 | Silent fallback to random DocumentKey on ECDH failure | **OPEN** — `sync/lib.rs:2202-2210`, `unwrap_or_else(\|_\| DocumentKey::generate())` unchanged. |
| H4 | Passphrases not zeroized in stub handlers | **OPEN** — `handle_change_passphrase`/`handle_recover_account` are still `let _ = (...)`. |
| H5 | Passphrases/recovery words plaintext across IPC | **OPEN** — IPC types in `bus/rest.rs` still carry bare `Vec<u8>`/`String`. |
| H6 | Remote CRDT edits silently dropped (`apply_remote` stub) | **RESOLVED** — `trame/src/logic/sync.rs` implements `apply_remote`/`apply_remote_batch` via Loro `import` + reseed. |
| H7 | Stale formula cache on plain-text conversion | **RESOLVED** — `on_cell_changed` evicts the cache entry when a cell is cleared; `cell_text` gates on `is_formula_cell()`. |
| M6 | Rate limiter fail-open + fixed-window burst | **OPEN** — still fail-open (`allowing request`) and a fixed-window counter. |
| M7 | OIDC discovery failure silently ignored | **OPEN** — `main.rs` still `.ok().unwrap_or_default()`. |
| M8 | `ambre-logic` persistence.rs dead code | **RESOLVED** — `pub mod persistence;` is now declared in `lib.rs`. |
| M9 | No HTTP timeouts in Sync thread | **OPEN** — `sync/http.rs` still sets no per-request timeout. |
| M10 | `WsError` does not trigger reconnection | **OPEN** — `sync/lib.rs:1466` still only logs; no `schedule_reconnect()`. |

The stub-related MEDIUM/OTHER findings from the previous pass (channel handlers,
meeting handlers, Murmure/Rendez-Vous) remain accurate: those subsystems are
still `tracing::info!`-only no-ops, which is expected — they are `Todo/` specs.

---

## CRITICAL (2)

### C4 — Cross-document snapshot access (IDOR) in `snapshot_download_url`

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/routes/sync.rs:172-203`, `db.rs get_snapshot`

The handler authorizes membership on the `{document_id}` from the URL path, then
looks up the snapshot **by its own id only**:

```rust
db::get_document_member(&state.db, &doc_id, &s.user_id).await?.ok_or(Forbidden)?;
let snap = db::get_snapshot(&state.db, &snap_id).await?...;   // no doc_id check
let url = s3::generate_presigned_download_url(..., &snap.s3_key, ...);
```

`get_snapshot` runs `WHERE id = $1` and never checks `document_id`. A member of
document A can pass A's id in the path (passing the ACL) and **any** snapshot
UUID belonging to document B in the body, and receive a presigned GET URL for
B's blob. The same shape exists conceptually for any "authorize on path id,
fetch by body id" pair — audit `snapshot_metadata` and the delta paths for the
same pattern.

**Impact, honestly scoped.** The blob is E2EE: the attacker gets ciphertext
sealed under B's DocumentKey, which they don't hold, so no plaintext leaks.
But this is still a broken object-level authorization: it leaks existence,
size, and the encrypted bytes of documents the caller has no relationship with,
and it defeats the "blind relay only serves what you're a member of" invariant
the server is supposed to enforce. Relying on E2EE to contain an authz bug is
luck, not defense.

**Fix:** `get_snapshot` (and every by-id blob fetch) must take the `document_id`
and filter on it: `WHERE id = $1 AND document_id = $2`, returning 404 on
mismatch.

---

### C5 — Path traversal via unvalidated `content_hash` into S3 keys

**Crate:** `ambre-server`
**Files:** `crates/ambre-server/src/s3.rs:66-72`, `routes/media.rs` (all three handlers)

```rust
pub fn format_s3_media_key(env_prefix, document_id, content_hash: &str) -> String {
    format!("{env_prefix}/documents/{document_id}/media/{content_hash}")
}
```

`content_hash` is taken verbatim from the client request body and interpolated
straight into the S3 object key, in `media_upload_url`, `media_download_url` and
`media_confirm_upload`. Nothing validates it. The spec (`server §`, line ~333)
states `content_hash` MUST be a 64-char BLAKE2b-256 hex string, but that
contract is enforced **nowhere in the code**.

A caller who is a member of document A can send
`content_hash = "../<docB>/media/<hash>"` (or `../../snapshots/<id>`) and obtain
a presigned URL that points outside A's media prefix — reading or, via the
upload URL, **writing** an object under another document's prefix. Combined with
C4, this widens the blast radius of the blind-relay bypass.

**Impact.** Again bounded by E2EE for confidentiality of existing blobs, but the
**upload** URL is worse: an attacker can plant arbitrary ciphertext at a chosen
key another document will later read, or clobber a media object. Content-
addressing gives integrity only if the server actually verifies the address.

**Fix:** validate `content_hash` against `^[0-9a-f]{64}$` before it touches any
key builder; reject otherwise. Do the equivalent for any other client-supplied
path component. Ideally, have the server compute/verify the BLAKE2b-256 of the
uploaded bytes rather than trusting the client's hash at all.

---

## HIGH (6)

### H8 — Revoked member retains write access on a live WebSocket

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/ws/handler.rs:271-289 (subscribe), 300-405 (update)`

Membership is checked once, at `Subscribe` (`verify_document_access`). After that
`handle_update` only checks the in-memory `conn.doc_subscriptions.contains(&doc_id)`
set — it never re-consults `document_members`. If an owner revokes a member
(`revoke_from_document`) while that member holds an open, already-subscribed
socket, the member keeps writing accepted, signed, sequenced updates until they
disconnect. This compounds the previously-noted "WS session never re-validated"
item, but the authorization dimension (write ACL, not just token expiry) makes it
HIGH.

**Mitigation already present (partial):** after revocation the owner rotates the
DocumentKey, so the revoked member's updates are sealed under the *old* key and
other members drop them on commitment mismatch. That contains data leakage into
the shared state, but (a) rotation is a separate client-initiated call that may
lag or fail, and (b) the revoked member can still consume seq numbers and cause
churn. Defense-in-depth is missing.

**Fix:** on `revoke`/`rotate`, push a targeted close (or a "resubscribe" nudge)
to the revoked user's connections; and/or re-check membership in `handle_update`
on a short TTL cache rather than trusting the subscription set indefinitely.

---

### H9 — Unbounded zstd decompression on the peer-fed sync path (WASM OOM)

**Crate:** `ambre-sync`
**File:** `crates/ambre-sync/src/pipeline.rs:63-76`

```rust
let mut decoder = ruzstd::decoding::StreamingDecoder::new(body)?;
let mut out = Vec::new();
decoder.read_to_end(&mut out)?;   // no cap
Ok(out)
```

`decompress` reads the entire zstd stream into memory with no size limit. This
runs *after* AEAD/commitment/signature verification, but the DocumentKey is
shared with every collaborator, so a **malicious-but-authorized** peer can craft
an Update/Snapshot whose plaintext is a zstd bomb (zstd reaches ~1000:1+). A few
KB on the wire decompresses to gigabytes and OOMs the victim's WASM Sync worker.
The previous audit flagged an *unbounded RESP parser recursion on the server*;
this is the analogous, arguably worse, hazard on the **client** because it's
reachable by any co-editor of a shared document.

**Fix:** cap the decompressed size (e.g. read in a bounded loop into a
`Vec::with_capacity(limit)` and error past a per-message ceiling derived from the
compressed length and a max ratio). Reject before allocating the full output.

---

### H2 — Document metadata leaked to non-members *(carried over, still open)*

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/routes/documents.rs:295-319`

When `get_document_member` returns `None`, the handler still returns
`id`, `owner_id`, `doc_type`, `name_enc` (only `role`/`doc_key_enc` are gated).
`name_enc` is ciphertext so the name itself is safe, but leaking `owner_id` and
`doc_type` to any authenticated (or, on this route, even anonymous) caller with a
document UUID is an unnecessary metadata disclosure.

**Fix:** return 404 when `mem` is `None` for a caller who isn't the owner.

---

### H3 — Silent fallback to a random DocumentKey on unwrap failure *(carried over)*

**Crate:** `ambre-sync`
**File:** `crates/ambre-sync/src/lib.rs:2200-2210`

`unwrap_document_key(...).unwrap_or_else(|_| DocumentKey::generate())` still turns
a decryption failure (wrong exchange key, corrupted server row, or an *attack*)
into a fresh random key. The document opens "successfully" and every subsequent
decrypt fails with a confusing commitment mismatch instead of a clear auth error.
Worse for diagnosis: this hides the difference between "I'm not really a member"
and "someone tampered with my wrapped key".

**Fix:** propagate the failure as an `AuthError` to the Logic thread; never
substitute a random key for a failed unwrap.

---

### H4 / H5 — Passphrase & recovery-word material not zeroized *(carried over)*

**Crates:** `ambre-sync`, `ambre-bus`
**Files:** `sync/lib.rs:1168-1176`, `bus/rest.rs:129-153`

```rust
fn handle_change_passphrase(&mut self, current: Vec<u8>, new: Vec<u8>) {
    tracing::info!("Sync: ChangePassphrase");
    let _ = (current, new);   // dropped without zeroization
}
```

Both handlers are still stubs that bind-and-drop secret bytes without wiping
them; the IPC types (`RestCommand::Unlock/SetupAccount/ChangePassphrase/
RecoverAccount`) still move bare `Vec<u8>`/`String` through the Logic thread. The
BIP39 recovery mnemonic — equivalent to the master key — travels as a plain
`String`. Note the doc comment on `bus/rest.rs:132` *claims* "zeroized by the
Sync thread after use", which is currently false; the comment is aspirational.

**Fix:** `Zeroizing<Vec<u8>>` in the IPC types; explicit `.zeroize()` in every
handler that consumes them, including the stubs.

---

## MEDIUM (9)

### M13 — Ed25519 verification is non-strict (signature malleability)

**Crate:** `ambre-crypto`
**File:** `crates/ambre-crypto/src/signature.rs:38-52`

`verify` uses `VerifyingKey::verify` (the permissive path), not `verify_strict`.
ed25519-dalek's default verification accepts non-canonical `S` and small-order
`A`, which means a third party can transform a valid `(R,S)` into a distinct
valid signature over the *same* message. Ambre uses the signature as the author-
attribution primitive on the blind relay, and pairs it with a monotonic per-
author clock. Malleability doesn't let an attacker forge a *new* message, but in
a system that treats "signature identifies the author" as load-bearing, allowing
signature variants is a footgun (e.g. any downstream dedup/equality on signature
bytes breaks, and it weakens non-repudiation reasoning).

**Recommendation:** switch to `verify_strict` for `Update`/`Snapshot`/`Ephemeral`
verification, both client-side (`envelope::open`) and server-side
(`ws/handler.rs verify_update_signature`). Low cost, removes a whole class of
"two valid signatures for one message" reasoning hazards.

---

### M14 — ECDH KEK derivation omits identity binding

**Crate:** `ambre-crypto`
**File:** `crates/ambre-crypto/src/ecdh.rs:33-43`

```rust
hasher.update(KEK_DOMAIN);
hasher.update(shared);      // = X25519(own, peer)
```

The key-encryption key is derived from `domain || shared_secret` only. The two
parties' public keys are **not** mixed in. This is not exploitable on its own
(the shared secret already binds both keys), but it deviates from the standard
HKDF-with-`info = pk_sender ‖ pk_recipient` construction and removes the natural
transcript binding you'd want if the wrap scheme ever grows (multiple recipients,
key confirmation, rewrapping). Additionally, `shared_secret` does not reject an
all-zero output (contributory behaviour): a peer public key of low order yields a
predictable shared secret. x25519-dalek doesn't reject these by default.

**Recommendation:** derive the KEK via HKDF-BLAKE2b with
`info = KEK_DOMAIN ‖ sender_pk ‖ recipient_pk`, and reject an all-zero shared
secret (`CryptoError::WeakPublicKey`). Cheap, and it future-proofs the wrap path.

---

### M15 — JWT access token carried in the WebSocket query string

**Crate:** `ambre-server`
**Files:** `ws/handler.rs:50-78`, `main.rs:315` (`TraceLayer::new_for_http()`)

The access JWT is passed as `GET /api/ws?token=<jwt>`. This is a documented,
justified choice (the browser WS API forbids custom headers — spec `server §478`).
The residual risk the spec does *not* call out: query strings land in reverse-
proxy access logs, browser history, and `Referer`, and `TraceLayer` will log the
request URI at DEBUG. A short-lived access token limits the window, but it's
still a bearer credential in a logged location.

**Recommendation:** document the operational requirement to strip `?token=` from
access logs at the proxy; consider a one-time WS ticket (short-TTL Redis nonce
minted over an authenticated POST, exchanged in the query) so the long-lived JWT
never appears in a URL.

---

### M16 — `get_deltas` limit is unbounded

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/routes/sync.rs:210,237`

```rust
let limit = params.limit.unwrap_or(100);   // no upper clamp
```

A client can request `?limit=9223372036854775807`, forcing a full-table scan of
`updates` for a document and a large response allocation. Not catastrophic (row-
bounded by the document's history) but it's a trivial amplification vector.

**Fix:** `let limit = params.limit.unwrap_or(100).clamp(1, 1000);`.

---

### M6 — Rate limiter fail-open + fixed-window burst *(carried over)*

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/middleware/rate_limit.rs:131-197`

Still fail-open (any Redis error → `allowing request`), still a fixed-window
counter (2× burst across the boundary), still `INCR` then a separate `EXPIRE`
(a crash between them leaves a TTL-less key). See original writeup.

**Fix:** sliding-window (sorted-set) counters; `SET key 1 EX ttl NX` for atomic
TTL; a fail-closed degraded mode for the auth tier.

---

### M17 — Permissive CORS (`Any` origin + `Any` headers)

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/main.rs:191-194`

```rust
CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any)
```

Because auth is a `Bearer` token in memory (no cookies), this is **not** a CSRF
hole today — a cross-origin page can't read the response, and there's no ambient
credential to ride. But `allow_origin(Any)` is broader than a self-hostable,
sovereignty-focused product wants as a default, and it forecloses ever moving the
refresh token to a cookie without a rework.

**Recommendation:** drive allowed origins from config (the self-hoster's domain),
default-deny. Keep `Any` only behind an explicit dev flag.

---

### M7 — OIDC discovery failure silently swallowed at startup *(carried over)*
### M9 — No HTTP timeouts in the Sync thread *(carried over)*
### M10 — `WsError` does not trigger reconnection *(carried over)*

Unchanged from the previous audit; see that pass for the writeups and fixes.

---

## OTHER NOTABLE FINDINGS

### ambre-crypto

- **`MasterSeed::as_bytes` is `pub`** (`keys.rs:66`): exposes the raw root seed
  (recovery-word equivalent) to any caller of the crate. Restrict to `pub(crate)`
  or gate behind an explicit `recovery` API.
- **`MasterSeed` and `DocumentKey` derive `Clone`** (`keys.rs:35,72`): silent
  duplication of secret material; each clone is a separate copy to zeroize.
  Carried over — still present.
- **Argon2id parameters** (`passphrase.rs:14-17`): 64 MiB / 3 passes / **1 lane**.
  Memory/passes are reasonable for a 2024 mobile SoC; the single lane is the weak
  axis. Re-confirm against `Cargo.lock` and consider bumping parallelism where the
  target can afford it. Params are versioned (good) so this can evolve safely.
- **AEAD `seal` panics via `.expect`** (`aead.rs:24`): "infallible for valid
  keys" is true for XChaCha20-Poly1305, but a panic in the Sync worker is still a
  hard failure mode; returning a `CryptoError` is cheap insurance.
- **No length-prefix between `ciphertext` and `aad` in the signed message**
  (`signature.rs:14-21`): the boundary is positional. Not exploitable given fixed-
  layout fields, but a length-prefixed transcript is more robust to future field
  additions.

### ambre-server

- **H1 residue**: `append_pending_delta` and `doc_broadcast` failures are still
  swallowed (`let _ =`) after the seq is irreversibly assigned — a Redis blip
  drops the delta from the flush list (never reaches S3) with no error surfaced.
  Log at ERROR + dead-letter.
- **`setup_account` hardcoded identity** (`documents.rs`): `temp@ambre.app` /
  `"User"`, rate-limited only by the global auth tier — account-creation spam
  vector. Carried over.
- **Clock hash unbounded growth** (`redis_ops.rs`): per-author entries in
  `doc:{id}:clocks` are never pruned; a document edited by many ephemeral authors
  grows the hash without bound. Carried over.
- **Flusher partial-failure re-upload** (`s3.rs`): on a mid-batch error the code
  `break`s and skips the `ltrim`, so already-flushed entries are re-processed next
  cycle. Carried over.
- **53 `.unwrap()/.expect()` in non-test server code**: most are startup/encode
  invariants, but audit the request-path ones (e.g. `postcard::to_allocvec(...).
  expect("postcard encode Update")` in `ws/handler.rs`) — a malformed value there
  panics the connection task.
- **`snapshot_metadata` shares C4's shape** (`sync.rs`): authorize on path id,
  return `s3_key` — verify it can't be pivoted the same way once C4 is fixed.

### ambre-sync

- **Channel & meeting handlers remain empty stubs** (`lib.rs:823-863`):
  `ListChannels`/`CreateChannel`/`DeleteChannel`/`CreateMeeting`/`JoinMeeting`
  only `tracing::info!`; server channel data is discarded. Expected (Murmure/
  Rendez-Vous are `Todo/`), but the bus surface exists and looks live.
- **Ephemeral remote counters unbounded** (`lib.rs:329`):
  `HashMap<[u8; 32], u64>` per document, no eviction. Carried over.
- **AuthError messages leak crypto detail** (key lengths, algorithm names):
  carried over.

### ambre-trame (largest crate — 24.7k LOC)

The formula-engine and CRDT-logic gaps from the previous pass are worth
re-checking now that `apply_remote` (H6) landed:

- **Range registration O(range_size)** (`formula/dag.rs`): `=SUM(A1:ZZ100000)`
  still iterates the full range at registration. With remote apply now live, a
  peer can push such a formula and burn CPU on every collaborator. Bound it.
- **`flush_delta` swallows Loro export errors** via `.ok()` — a failed export
  silently drops the local delta. Now that remote apply works, a dropped local
  delta means permanent divergence, not just a missed frame.
- Duplicated `col_to_alpha` (×4) and glyph-draw loops (×5): maintainability debt,
  unchanged.
- `LEFT`/`RIGHT` negative-length `as usize` wrap, `Pow` NaN/inf passthrough,
  `as u32` on negative floats: carried over, still worth the guards.

### ambre-canvas

- **M1–M5 (matchMedia DPI, GPU context-loss, atlas/text-cache all-or-nothing
  eviction, stroke-corner double-blend)**: re-verified, all **still open**.
  `text_cache.rs:82` `evict_if_oversized` still does `self.layouts.clear()`;
  `render/mod.rs` still full-clears the glyph atlas on overflow; `stroke_rect`
  (`render/mod.rs:1251`) still draws 4 overlapping quads (double-blend on
  translucent strokes); no `webglcontextlost`/`device_lost` handling anywhere.

---

## CORRECTIONS TO THE PREVIOUS AUDIT

Rigour cuts both ways — one prior finding does not hold up:

- **C3 (timing side-channel in HMAC commitment verification) is a FALSE
  POSITIVE.** The claim was that `Hmac::verify_slice` in `hmac ≤ 0.12.1` uses a
  non-constant-time byte comparison. It does not: `verify_slice` compares via
  `CtOutput`, whose `PartialEq` is implemented in constant time using the
  `subtle` crate. There is no RUSTSEC advisory to this effect. `commitment.rs`
  as written is correct; no change is required. (The general advice to pin
  crypto deps in `Cargo.lock` still stands — but the lockfile wasn't in this
  snapshot to confirm.)

---

## POSITIVE FINDINGS

Confirmed against the current tree:

- **Cross-instance sync now works** — `PubSubHub` (`ws/pubsub_hub.rs`) is the
  missing subscriber from C2, correctly one-subscriber-per-channel fanned out
  in-process.
- **Remote collaborative apply is real** — Trame's `apply_remote`/
  `apply_remote_batch` import Loro deltas and reseed the viewport (H6 closed).
- **Atomic, strict clock+seq** — the Lua script enforces `incoming == current+1`
  exactly, atomically, rejecting both rollback and gaps. No TOCTOU.
- **Envelope order is correct** — seal: AEAD → HMAC commitment → Ed25519 sign;
  open: verify signature → verify commitment → AEAD open. The signature covers
  the commitment tag, so the relay cannot substitute it.
- **Ed25519 signature is verified server-side before seq assignment** — the blind
  relay authenticates the author before touching state.
- **Domain separation** on every derivation (`SIGN`/`EXCH`/`KEK` domains).
- **`ZeroizeOnDrop`** on `MasterSeed` and `DocumentKey`.
- **CSPRNG nonces** (192-bit XChaCha20 via `getrandom`, mapped to
  `crypto.getRandomValues` on WASM).
- **Ephemeral anti-replay** — per-author monotonic counter drops replayed
  presence messages.
- **Zero SQL injection** — every query uses `sqlx::query*()` with `.bind()`.
- **Exactly one `unsafe`** in the whole codebase (`core/version.rs
  new_unchecked`), with an explicit documented safety contract. `ambre-crypto`,
  `ambre-canvas`, `ambre-bus` remain `unsafe`-free.
- **`ambre-crypto` is NOT a dependency of `ambre-server`** — the server verifies
  Ed25519 via `ed25519-dalek` directly and never links `DocumentKey`/`MasterSeed`/
  Argon2 into its process. Least-privilege boundary held (spec `server §948`).
- **476 test functions** across unit/integration/wasm targets.

---

## REMEDIATION ROADMAP

Security-first ordering, grouped into waves. Same `[ ]`/`[x]` convention as
`ambre-roadmap.md` — a wave is `[x]` only when every story under it is `[x]`.
Each story carries its finding ID, the file to touch, and the concrete action.

### Wave R0 — Authorization holes (do first; small diffs, real bypasses)

- [ ] **C4** — `db::get_snapshot` takes `document_id` and filters
      `WHERE id = $1 AND document_id = $2`; `snapshot_download_url` passes the
      path `doc_id`, returns 404 on mismatch. (`server/db.rs`, `routes/sync.rs`)
- [ ] **C4-audit** — sweep every "authorize on path id, fetch by body id" pair
      for the same shape: `snapshot_metadata`, delta fetch, media by-hash.
      (`server/routes/sync.rs`, `routes/media.rs`)
- [ ] **C5** — validate `content_hash` against `^[0-9a-f]{64}$` before any key
      builder; reject otherwise. Apply to all three media handlers.
      (`server/routes/media.rs`, `s3.rs`)
- [ ] **C5-hardening** — server computes/verifies BLAKE2b-256 of uploaded bytes
      instead of trusting the client hash (defeats content-address spoofing).
      (`server/routes/media.rs`)

### Wave R1 — Peer-triggered DoS & live-session authz

- [ ] **H9** — bound zstd output: read in a loop into
      `Vec::with_capacity(limit)`, error past a per-message ceiling derived from
      `body.len()` × max-ratio. Reject before allocating. (`sync/pipeline.rs`)
- [ ] **H8** — on `revoke`/`rotate`, push a targeted close (or resubscribe nudge)
      to the revoked user's connections; add a short-TTL membership re-check in
      `handle_update` rather than trusting `doc_subscriptions` indefinitely.
      (`server/ws/handler.rs`, `routes/share.rs`)
- [ ] **M16** — `params.limit.unwrap_or(100).clamp(1, 1000)` in `get_deltas`.
      (`server/routes/sync.rs`)

### Wave R2 — Make the E2EE story honest (fail-closed + secret hygiene)

- [ ] **H3** — replace `unwrap_or_else(|_| DocumentKey::generate())` with an
      `AuthError` propagated to the Logic thread; never substitute a random key
      for a failed unwrap. (`sync/lib.rs:2200-2210`)
- [ ] **H4** — zeroize secret bytes in `handle_change_passphrase` /
      `handle_recover_account` (explicit `.zeroize()`, drop the `let _ =`).
      (`sync/lib.rs:1168-1176`)
- [ ] **H5** — `Zeroizing<Vec<u8>>` / `Zeroizing<String>` in the IPC types
      (`Unlock`/`SetupAccount`/`ChangePassphrase`/`RecoverAccount`); make the
      `bus/rest.rs:132` "zeroized after use" comment true. (`bus/rest.rs`, `sync/lib.rs`)
- [ ] **H2** — return 404 from `get_document` when `mem == None` and the caller
      isn't the owner. (`server/routes/documents.rs:295-319`)
- [ ] **C1** — redact `blob_str` in the S3 flusher error logs; keep only `seq`,
      `document_id`, `s3_key`. (`server/s3.rs:215,223`)

### Wave R3 — Crypto hardening (cheap, while the surface is small)

- [ ] **M13** — switch to `verify_strict` in `envelope::open` and
      `verify_update_signature`. (`crypto/signature.rs`, `server/ws/handler.rs`)
- [ ] **M14** — derive the KEK via HKDF-BLAKE2b with
      `info = KEK_DOMAIN ‖ sender_pk ‖ recipient_pk`; reject an all-zero shared
      secret (`CryptoError::WeakPublicKey`). (`crypto/ecdh.rs`)
- [ ] **crypto-hygiene** — restrict `MasterSeed::as_bytes` to `pub(crate)` (or a
      dedicated recovery API); reconsider `Clone` on `MasterSeed`/`DocumentKey`;
      return `CryptoError` from `aead::seal` instead of `.expect`. (`crypto/keys.rs`, `aead.rs`)
- [ ] **argon2-review** — confirm params against the real `Cargo.lock`; bump lane
      count where the target affords it. (`crypto/passphrase.rs`)

### Wave R4 — Robustness / operational

- [ ] **H1** — log `append_pending_delta` / `doc_broadcast` failures at ERROR;
      add a dead-letter path for a delta that never reached S3. (`server/ws/handler.rs`)
- [ ] **M6** — sliding-window rate limiter (sorted sets); `SET key 1 EX ttl NX`
      for atomic TTL; fail-closed degraded mode on the auth tier. (`server/middleware/rate_limit.rs`)
- [ ] **M7** — log OIDC discovery failure at ERROR; fail-fast on misconfig.
      (`server/main.rs`)
- [ ] **M9** — 30s default timeout on all Sync HTTP calls (AbortController /
      gloo-net timeout). (`sync/http.rs`)
- [ ] **M10** — `WsError` triggers the same `schedule_reconnect()` as
      `WsDisconnected`. (`sync/lib.rs:1466`)
- [ ] **M15** — document proxy log-stripping of `?token=`; consider a one-time WS
      ticket so the JWT never appears in a URL. (`server/ws/handler.rs`, deploy docs)
- [ ] **M17** — config-driven CORS allow-list, default-deny; `Any` only behind a
      dev flag. (`server/main.rs:191`)

### Wave R5 — Maintainability debt (no security impact)

- [ ] **trame-formula** — bound `formula/dag.rs` range registration (currently
      O(range_size); now reachable via remote apply); stop `.ok()`-swallowing
      Loro export errors in `flush_delta`. (`trame/src/logic/sync.rs`, `formula/dag.rs`)
- [ ] **trame-dedup** — collapse the ×4 `col_to_alpha` and ×5 glyph-draw copies;
      add guards for `LEFT`/`RIGHT` negative length, `Pow` NaN/inf, negative-float
      `as u32`. (`trame/src/**`)
- [ ] **canvas-eviction** — weighted-LRU eviction for the glyph atlas and text
      cache instead of full-clear (M3/M4). (`canvas/render/mod.rs`, `text_cache.rs`)
- [ ] **canvas-gpu** — `webglcontextlost`/`webglcontextrestored` + device
      re-creation; fix the `matchMedia` DPI listener (M1/M2). (`canvas/host.rs`)
- [ ] **canvas-stroke** — single mitered quad strip for `stroke_rect` to kill the
      translucent-corner double-blend (M5). (`canvas/render/mod.rs:1251`)
- [ ] **server-hygiene** — prune the `doc:{id}:clocks` hash; fix the flusher
      partial-failure re-upload (`ltrim` skipped on `break`); audit request-path
      `.expect()` calls. (`server/redis_ops.rs`, `s3.rs`, `ws/handler.rs`)
