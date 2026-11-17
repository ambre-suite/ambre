# AMBRE -- CODE AUDIT

> Full-codebase audit across 16 crates, ~51 000 lines of Rust. Findings grouped by severity.
> **Date:** 2026-07-05 (supersedes the 2026-06-07 pass on 7 crates / ~15 000 lines)
>
> **State, 2026-08-27.** Every numbered finding below is closed. The report is kept as written; the
> ledger in "Status of previous findings" carries the state, and names the contract row that holds
> each fix.
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
| C1 | Ciphertext logged in S3 flusher error paths | **OPEN** -- `s3.rs:215,223` still log `blob_str` (contains `ciphertext_b64`). |
| C2 | Missing Redis PubSub subscriber (cross-instance sync broken) | **RESOLVED** -- `ws/pubsub_hub.rs` (`PubSubHub::spawn`, subscribe/unsubscribe fan-out) is wired in `main.rs`. |
| C3 | Timing side-channel in HMAC commitment verification | **FALSE POSITIVE** -- see "Corrections to the previous audit" below. `Mac::verify_slice` is constant-time. |
| H1 | WS handler silently drops DB/Redis errors | **PARTIAL** -- `insert_update` now logs at ERROR; `append_pending_delta` and `doc_broadcast` are still `let _ =`. |
| H2 | Document metadata leaked to non-members | **OPEN** -- `documents.rs` `get_document` still returns `id`/`owner_id`/`doc_type`/`name_enc` when `mem == None`. |
| H3 | Silent fallback to random DocumentKey on ECDH failure | **OPEN** -- `sync/lib.rs:2202-2210`, `unwrap_or_else(\|_\| DocumentKey::generate())` unchanged. |
| H4 | Passphrases not zeroized in stub handlers | **OPEN** -- `handle_change_passphrase`/`handle_recover_account` are still `let _ = (...)`. |
| H5 | Passphrases/recovery words plaintext across IPC | **OPEN** -- IPC types in `bus/rest.rs` still carry bare `Vec<u8>`/`String`. |
| H6 | Remote CRDT edits silently dropped (`apply_remote` stub) | **RESOLVED** -- `trame/src/logic/sync.rs` implements `apply_remote`/`apply_remote_batch` via Loro `import` + reseed. |
| H7 | Stale formula cache on plain-text conversion | **RESOLVED** -- `on_cell_changed` evicts the cache entry when a cell is cleared; `cell_text` gates on `is_formula_cell()`. |
| M6 | Rate limiter fail-open + fixed-window burst | **OPEN** -- still fail-open (`allowing request`) and a fixed-window counter. |
| M7 | OIDC discovery failure silently ignored | **OPEN** -- `main.rs` still `.ok().unwrap_or_default()`. |
| M8 | `ambre-logic` persistence.rs dead code | **RESOLVED** -- `pub mod persistence;` is now declared in `lib.rs`. |
| M9 | No HTTP timeouts in Sync thread | **OPEN** -- `sync/http.rs` still sets no per-request timeout. |
| M10 | `WsError` does not trigger reconnection | **OPEN** -- `sync/lib.rs:1466` still only logs; no `schedule_reconnect()`. |

### Re-verified against the tree, 2026-08-27

The column above is dated and stays as written, and it is no longer the state: every numbered
finding of this report is closed today. A reader of that column alone acts on findings that the
remediation waves removed. This ledger is the state; each section below is kept as written, because
the reasoning is what a reader needs the day the same shape appears elsewhere.

`Held by` names the contract row that goes red if the fix is reverted. A finding that states no
behaviour rule holds none, and `scripts/contracts.py` carries the reason in its `NO_RULE` register.
The script reads this ledger from both ends: a row cannot cite a finding this report does not
declare, and a finding cannot sit here uncited.

| ID | Now | Closed by | Held by |
| :-- | :-- | :-- | :-- |
| C1 | closed | wave R2 -- the flusher error paths log `blob_len`, never `blob_str` | `ZK-006` |
| C2 | closed | before this pass -- `ws/pubsub_hub.rs` subscribes and fans out per process | `SRV-073` |
| C3 | false positive | the comparison it reports is already constant-time | none, and by decision |
| C4 | closed | wave R0 -- a snapshot is fetched by its id and its document | `SRV-004`, `SRV-054` |
| C5 | closed | wave R0 -- a content hash is refused unless it is sixty-four hexadecimal characters | `SRV-054` |
| H1 | closed | wave R4 -- a failed append is logged and dead-lettered for replay | `SRV-033`, `SRV-034` |
| H2 | closed | wave R2 -- a non-member reads a not-found answer | `SRV-005` |
| H3 | closed | wave R2 -- a failed unwrap surfaces an authentication error, and generates no key | `SYNC-012` |
| H4 | closed | wave R2 -- both handlers take a type that wipes itself | `ZK-010` |
| H5 | closed | wave R2 -- the bus carries `SecretBytes` and `SecretString` | `ZK-010` |
| H6 | closed | before this pass -- the engine applies a remote payload and reseeds | `TRAME-015` |
| H7 | closed | before this pass -- a cell cleared of its formula drops its cache entry | `TRAME-038` |
| H8 | closed | wave R1 -- membership is re-read on a thirty-second ceiling | `SRV-043` |
| H9 | closed | wave R1 -- decompression stops at a ceiling drawn from the compressed length | `SYNC-009` |
| M6 | closed | wave R4 -- one sliding window, and the sign-in tier fails closed | `SRV-009`, `SRV-013`, `SRV-019` |
| M7 | closed | wave R4 -- a configured provider that does not answer stops the start | `SRV-074` |
| M8 | closed | before this pass -- the module is declared, and it states no behaviour rule | none, and by decision |
| M9 | closed | wave R4 -- every request carries a thirty-second abort signal | `SYNC-071` |
| M10 | closed | wave R4 -- a socket error takes the teardown and backoff path | `SYNC-070` |
| M13 | closed | wave R3 -- both verifiers take the strict form | `SYNC-072` |
| M14 | closed | wave R3 -- the key derivation binds the identity pair, and refuses a weak key | `SYNC-035`, `SYNC-073` |
| M15 | closed | wave R4 -- the request span records the path alone | `SRV-031` |
| M16 | closed | wave R1 -- the count a caller asks for is clamped to a thousand | `SRV-075` |
| M17 | closed | wave R4 -- the origin list comes from the deployment, and defaults to none | `SRV-068` |
| N1 | closed | wave R6 -- recovery is the owner's alone, and a threshold above one is refused | `SRV-062`, `SRV-065` |
| N2 | closed | wave R6 -- a granted role is clamped to the caller's own | `SRV-016`, `SRV-061`, `SRV-070`, `SRV-071` |
| N3 | closed | wave R6 -- the account carries the address the client supplies, bounded | `SRV-029` |
| N4 | closed | wave R7 -- neither request type declares a passphrase field | `ZK-012` |
| N5 | closed | wave R7 -- one address, one account, whatever the letter case | `SRV-028` |

The stub-related MEDIUM/OTHER findings from the previous pass (channel handlers,
meeting handlers, Murmure/Rendez-Vous) remain accurate: those subsystems are
still `tracing::info!`-only no-ops, which is expected -- they are `Todo/` specs.

---

## CRITICAL (2)

### C4 -- Cross-document snapshot access (IDOR) in `snapshot_download_url`

> **Resolved since this pass**, see the re-verification above. The report below is kept as written,
> because the reasoning is what a reader needs the day the same shape appears elsewhere -- and the
> section itself names two places to check for it.

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
fetch by body id" pair -- audit `snapshot_metadata` and the delta paths for the
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

### C5 -- Path traversal via unvalidated `content_hash` into S3 keys

**Crate:** `ambre-server`
**Files:** `crates/ambre-server/src/s3.rs:66-72`, `routes/media.rs` (all three handlers)

```rust
pub fn format_s3_media_key(env_prefix, document_id, content_hash: &str) -> String { /* ... */ }
```

`content_hash` is taken verbatim from the client request body and interpolated
straight into the S3 object key, in `media_upload_url`, `media_download_url` and
`media_confirm_upload`. Nothing validates it. The spec (`server section `, line ~333)
states `content_hash` MUST be a 64-char BLAKE2b-256 hex string, but that
contract is enforced **nowhere in the code**.

A caller who is a member of document A can send
`content_hash = "../<docB>/media/<hash>"` (or `../../snapshots/<id>`) and obtain
a presigned URL that points outside A's media prefix -- reading or, via the
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

### H8 -- Revoked member retains write access on a live WebSocket

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/ws/handler/message.rs` (`handle_subscribe`, `handle_update`)

Membership is checked once, at `Subscribe` (`verify_document_access`). After that
`handle_update` only checks the in-memory `conn.doc_subscriptions.contains(&doc_id)`
set -- it never re-consults `document_members`. If an owner revokes a member
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

### H9 -- Unbounded zstd decompression on the peer-fed sync path (WASM OOM)

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

**Fix:** cap the decompressed size (for example read in a bounded loop into a
`Vec::with_capacity(limit)` and error past a per-message ceiling derived from the
compressed length and a max ratio). Reject before allocating the full output.

---

### H2 -- Document metadata leaked to non-members *(closed, see the ledger)*

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/routes/documents.rs:295-319`

When `get_document_member` returns `None`, the handler still returns
`id`, `owner_id`, `doc_type`, `name_enc` (only `role`/`doc_key_enc` are gated).
`name_enc` is ciphertext so the name itself is safe, but leaking `owner_id` and
`doc_type` to any authenticated (or, on this route, even anonymous) caller with a
document UUID is an unnecessary metadata disclosure.

**Fix:** return 404 when `mem` is `None` for a caller who isn't the owner.

---

### H3 -- Silent fallback to a random DocumentKey on unwrap failure *(closed, see the ledger)*

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

### H4 / H5 -- Passphrase & recovery-word material not zeroized *(closed, see the ledger)*

**Crates:** `ambre-sync`, `ambre-bus`
**Files:** `sync/lib.rs:1168-1176`, `bus/rest.rs:129-153`

```rust
fn handle_change_passphrase(&mut self, current: Vec<u8>, new: Vec<u8>) { /* ... */ }
```

Both handlers are still stubs that bind-and-drop secret bytes without wiping
them; the IPC types (`RestCommand::Unlock/SetupAccount/ChangePassphrase/
RecoverAccount`) still move bare `Vec<u8>`/`String` through the Logic thread. The
BIP39 recovery mnemonic -- equivalent to the master key -- travels as a plain
`String`. Note the doc comment on `bus/rest.rs:132` *claims* "zeroized by the
Sync thread after use", which is currently false; the comment is aspirational.

**Fix:** `Zeroizing<Vec<u8>>` in the IPC types; explicit `.zeroize()` in every
handler that consumes them, including the stubs.

---

## MEDIUM (9)

### M13 -- Ed25519 verification is non-strict (signature malleability)

**Crate:** `ambre-crypto`
**File:** `crates/ambre-crypto/src/signature.rs:38-52`

`verify` uses `VerifyingKey::verify` (the permissive path), not `verify_strict`.
ed25519-dalek's default verification accepts non-canonical `S` and small-order
`A`, which means a third party can transform a valid `(R,S)` into a distinct
valid signature over the *same* message. Ambre uses the signature as the author-
attribution primitive on the blind relay, and pairs it with a monotonic per-
author clock. Malleability doesn't let an attacker forge a *new* message, but in
a system that treats "signature identifies the author" as load-bearing, allowing
signature variants is a footgun (for example any downstream dedup/equality on signature
bytes breaks, and it weakens non-repudiation reasoning).

**Recommendation:** switch to `verify_strict` for `Update`/`Snapshot`/`Ephemeral`
verification, both client-side (`envelope::open`) and server-side
(`ws/handler.rs verify_update_signature`). Low cost, removes a whole class of
"two valid signatures for one message" reasoning hazards.

---

### M14 -- ECDH KEK derivation omits identity binding

**Crate:** `ambre-crypto`
**File:** `crates/ambre-crypto/src/ecdh.rs:33-43`

```rust
hasher.update(KEK_DOMAIN);
hasher.update(shared);      // = X25519(own, peer)
```

The key-encryption key is derived from `domain || shared_secret` only. The two
parties' public keys are **not** mixed in. This is not exploitable on its own
(the shared secret already binds both keys), but it deviates from the standard
HKDF-with-`info = pk_sender || pk_recipient` construction and removes the natural
transcript binding you'd want if the wrap scheme ever grows (multiple recipients,
key confirmation, rewrapping). Additionally, `shared_secret` does not reject an
all-zero output (contributory behaviour): a peer public key of low order yields a
predictable shared secret. x25519-dalek doesn't reject these by default.

**Recommendation:** derive the KEK via HKDF-BLAKE2b with
`info = KEK_DOMAIN || sender_pk || recipient_pk`, and reject an all-zero shared
secret (`CryptoError::WeakPublicKey`). Cheap, and it future-proofs the wrap path.

---

### M15 -- JWT access token carried in the WebSocket query string

**Crate:** `ambre-server`
**Files:** `ws/handler.rs:50-78`, `main.rs:315` (`TraceLayer::new_for_http()`)

The access JWT is passed as `GET /api/ws?token=<jwt>`. This is a documented,
justified choice (the browser WS API forbids custom headers -- `server section 3`).
The residual risk the spec does *not* call out: query strings land in reverse-
proxy access logs, browser history, and `Referer`, and `TraceLayer` will log the
request URI at DEBUG. A short-lived access token limits the window, but it's
still a bearer credential in a logged location.

**Recommendation:** document the operational requirement to strip `?token=` from
access logs at the proxy; consider a one-time WS ticket (short-TTL Redis nonce
minted over an authenticated POST, exchanged in the query) so the long-lived JWT
never appears in a URL.

---

### M16 -- `get_deltas` limit is unbounded

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

### M6 -- Rate limiter fail-open + fixed-window burst *(closed, see the ledger)*

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/middleware/rate_limit.rs:131-197`

Still fail-open (any Redis error -> `allowing request`), still a fixed-window
counter (2x burst across the boundary), still `INCR` then a separate `EXPIRE`
(a crash between them leaves a TTL-less key). See original writeup.

**Fix:** sliding-window (sorted-set) counters; `SET key 1 EX ttl NX` for atomic
TTL; a fail-closed degraded mode for the auth tier.

---

### M17 -- Permissive CORS (`Any` origin + `Any` headers)

**Crate:** `ambre-server`
**File:** `crates/ambre-server/src/main.rs:191-194`

```rust
CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any)
```

Because auth is a `Bearer` token in memory (no cookies), this is **not** a CSRF
hole today -- a cross-origin page can't read the response, and there's no ambient
credential to ride. But `allow_origin(Any)` is broader than a self-hostable,
sovereignty-focused product wants as a default, and it forecloses ever moving the
refresh token to a cookie without a rework.

**Recommendation:** drive allowed origins from config (the self-hoster's domain),
default-deny. Keep `Any` only behind an explicit dev flag.

---

### M7 -- OIDC discovery failure silently swallowed at startup *(closed, see the ledger)*
### M9 -- No HTTP timeouts in the Sync thread *(closed, see the ledger)*
### M10 -- `WsError` does not trigger reconnection *(closed, see the ledger)*

Carried from the previous audit, which holds the three writeups. Wave R4 closed all three.

---

## OTHER NOTABLE FINDINGS

> **State, 2026-08-27.** These items carry no number, so no ledger row names them. Waves R3 to R6
> closed or settled every one: two are a recorded WON'T DO -- pruning the clock hash, and the single
> Argon2 lane -- and the channel and meeting handlers stay stubs while Murmure and Rendez-Vous are
> `Todo/` specs. The reports below are kept as written.

### ambre-crypto

- **`MasterSeed::as_bytes` is `pub`** (`keys.rs:66`): exposes the raw root seed
  (recovery-word equivalent) to any caller of the crate. Restrict to `pub(crate)`
  or gate behind an explicit `recovery` API.
- **`MasterSeed` and `DocumentKey` derive `Clone`** (`keys.rs:35,72`): silent
  duplication of secret material; each clone is a separate copy to zeroize.
  Carried over -- still present.
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
  swallowed (`let _ =`) after the seq is irreversibly assigned -- a Redis blip
  drops the delta from the flush list (never reaches S3) with no error surfaced.
  Log at ERROR + dead-letter.
- **`setup_account` hardcoded identity** (`documents.rs`): `temp@ambre.app` /
  `"User"`, rate-limited only by the global auth tier -- account-creation spam
  vector. Carried over.
- **Clock hash unbounded growth** (`redis_ops.rs`): per-author entries in
  `doc:{id}:clocks` are never pruned; a document edited by many ephemeral authors
  grows the hash without bound. Carried over.
- **Flusher partial-failure re-upload** (`s3.rs`): on a mid-batch error the code
  `break`s and skips the `ltrim`, so already-flushed entries are re-processed next
  cycle. Carried over.
- **53 `.unwrap()/.expect()` in non-test server code**: most are startup/encode
  invariants, but audit the request-path ones (for example `postcard::to_allocvec(...).
  expect("postcard encode Update")` in `ws/handler.rs`) -- a malformed value there
  panics the connection task.
- **`snapshot_metadata` shares C4's shape** (`sync.rs`): authorize on path id,
  return `s3_key` -- verify it can't be pivoted the same way once C4 is fixed.

### ambre-sync

- **Channel & meeting handlers remain empty stubs** (`lib.rs:823-863`):
  `ListChannels`/`CreateChannel`/`DeleteChannel`/`CreateMeeting`/`JoinMeeting`
  only `tracing::info!`; server channel data is dropped. Expected (Murmure/
  Rendez-Vous are `Todo/`), but the bus surface exists and looks live.
- **Ephemeral remote counters unbounded** (`lib.rs:329`):
  `HashMap<[u8; 32], u64>` per document, no eviction. Carried over.
- **AuthError messages leak crypto detail** (key lengths, algorithm names):
  carried over.

### ambre-trame (largest crate -- 24.7k LOC)

The formula-engine and CRDT-logic gaps from the previous pass are worth
re-checking now that `apply_remote` (H6) landed:

- **Range registration O(range_size)** (`formula/dag.rs`): `=SUM(A1:ZZ100000)`
  still iterates the full range at registration. With remote apply now live, a
  peer can push such a formula and burn CPU on every collaborator. Bound it.
- **`flush_delta` swallows Loro export errors** via `.ok()` -- a failed export
  silently drops the local delta. Now that remote apply works, a dropped local
  delta means permanent divergence, not just a missed frame.
- Duplicated `col_to_alpha` (x4) and glyph-draw loops (x5): maintainability debt,
  unchanged.
- `LEFT`/`RIGHT` negative-length `as usize` wrap, `Pow` NaN/inf passthrough,
  `as u32` on negative floats: carried over, still worth the guards.

### ambre-canvas

- **M1-M5 (matchMedia DPI, GPU context-loss, atlas/text-cache all-or-nothing
  eviction, stroke-corner double-blend)**: re-verified, all **still open**.
  `text_cache.rs:82` `evict_if_oversized` still does `self.layouts.clear()`;
  `render/mod.rs` still full-clears the glyph atlas on overflow; `stroke_rect`
  (`render/mod.rs:1251`) still draws 4 overlapping quads (double-blend on
  translucent strokes); no `webglcontextlost`/`device_lost` handling anywhere.

---

## CORRECTIONS TO THE PREVIOUS AUDIT

Rigour cuts both ways -- one prior finding does not hold up:

- **C3 (timing side-channel in HMAC commitment verification) is a FALSE
  POSITIVE.** The claim was that `Hmac::verify_slice` in `hmac <= 0.12.1` uses a
  non-constant-time byte comparison. It does not: `verify_slice` compares via
  `CtOutput`, whose `PartialEq` is implemented in constant time using the
  `subtle` crate. There is no RUSTSEC advisory to this effect. `commitment.rs`
  as written is correct; no change is required. (The general advice to pin
  crypto deps in `Cargo.lock` still stands -- but the lockfile wasn't in this
  snapshot to confirm.)

---

## POSITIVE FINDINGS

Confirmed against the current tree:

- **Cross-instance sync now works** -- `PubSubHub` (`ws/pubsub_hub.rs`) is the
  missing subscriber from C2, correctly one-subscriber-per-channel fanned out
  in-process.
- **Remote collaborative apply is real** -- Trame's `apply_remote`/
  `apply_remote_batch` import Loro deltas and reseed the viewport (H6 closed).
- **Atomic, strict clock+seq** -- the Lua script enforces `incoming == current+1`
  exactly, atomically, rejecting both rollback and gaps. No TOCTOU.
- **Envelope order is correct** -- seal: AEAD -> HMAC commitment -> Ed25519 sign;
  open: verify signature -> verify commitment -> AEAD open. The signature covers
  the commitment tag, so the relay cannot substitute it.
- **Ed25519 signature is verified server-side before seq assignment** -- the blind
  relay authenticates the author before touching state.
- **Domain separation** on every derivation (`SIGN`/`EXCH`/`KEK` domains).
- **`ZeroizeOnDrop`** on `MasterSeed` and `DocumentKey`.
- **CSPRNG nonces** (192-bit XChaCha20 via `getrandom`, mapped to
  `crypto.getRandomValues` on WASM).
- **Ephemeral anti-replay** -- per-author monotonic counter drops replayed
  presence messages.
- **Zero SQL injection** -- every query uses `sqlx::query*()` with `.bind()`.
- **Exactly one `unsafe`** in the whole codebase (`core/version.rs
  new_unchecked`), with an explicit documented safety contract. `ambre-crypto`,
  `ambre-canvas`, `ambre-bus` remain `unsafe`-free.
- **`ambre-crypto` is NOT a dependency of `ambre-server`** -- the server verifies
  Ed25519 via `ed25519-dalek` directly and never links `DocumentKey`/`MasterSeed`/
  Argon2 into its process. Least-privilege boundary held (`server section 6`).
- **476 test functions** across unit/integration/wasm targets.

---

## ADDENDUM 2026-07-08 -- NEW SURFACE NOT IN THE 2026-07-05 PASS

The 2026-07-05 snapshot predates the **organizations / key-escrow** subsystem
(`routes/orgs.rs`, `routes/keys.rs`, `db/orgs.rs`, the org half of `routes/share.rs`).
It is now wired in `main.rs` (`/organizations`, `/organizations/{id}/recover/{uid}`,
`/keys/master`, `/keys/master/rotate`) and was never audited. Findings below.
Everything in Waves R0-R5 is confirmed still applied (all commits since
2026-07-05 are the remediation waves).

### N1 (HIGH) -- Org master-key recovery bypasses the advertised Shamir threshold

**Crate:** `ambre-server` -- **File:** `routes/orgs.rs:337-370`, `db/orgs.rs`

`recover_master_key` returns a target member's `org_master_key_enc` on the
authority of a **single** `owner`/`admin` (`require_org_role(... &["owner","admin"])`).
The schema persists `shamir_threshold` / `shamir_total` and the create/add-member
paths accept per-member `shamir_share_enc`, advertising a k-of-n escrow model, but
**no code path enforces threshold reconstruction** (grep: `threshold` appears only
in storage/echo, never in a check). The doc comment concedes Shamir "will be added
in a future iteration."

**Impact.** Confidentiality is bounded: the returned blob is sealed under the org
private key the caller must hold client-side, and each recovery is logged
(`log_org_recovery`). But a lone rogue/compromised admin can escrow-recover any
member's master key, defeating the k-of-n quorum the data model promises. For a
sovereignty/zero-knowledge product this is a governance deviation, not just a TODO.

**Fix:** either gate recovery behind real threshold reconstruction (collect k
shares before releasing anything), or, until Shamir lands, require the `owner`
role only and document loudly that recovery is single-authority. Reject
`shamir_threshold > 1` at org creation while unenforced, so the schema never
advertises a guarantee the server doesn't keep.

### N2 (MEDIUM) -- `invite_to_document` lets an editor mint co-owners

**Crate:** `ambre-server` -- **File:** `routes/share.rs:62,65`

The caller must be `owner` or `editor` (`["owner","editor"].contains(&m.role)`),
but the granted role is only checked against `["owner","editor","viewer"]` -- it is
**not** bounded by the caller's own role. An `editor` can therefore invite an
arbitrary user as `owner`; that new owner can then `revoke_from_document` /
`rotate_document_key` (both `owner`-only). Privilege escalation from editor to
(proxy) owner.

**Fix:** clamp the invitable role to `<= caller.role` (only an `owner` may grant
`owner`).

### N3 (LOW, carried) -- `setup_account` hardcoded placeholder identity *(closed, see the ledger)*

`routes/documents.rs:74-75` still creates accounts as `temp@ambre.app` / `"User"`,
rate-limited only by the global auth tier. Unchanged from the previous pass's
"OTHER NOTABLE FINDINGS"; no wave addressed it. Account-creation spam vector.

### Residuals from 2026-07-05 "OTHER NOTABLE FINDINGS" no wave covered

Four were listed here, and wave R6 closed three of them: `signature::message` frames each field with
its length, so the `ciphertext || aad` boundary is unambiguous; the sync `AuthError` messages carry
two generic strings and log the cause at WARN; the ephemeral counter map evicts the least-active
author at its ceiling. `MasterSeed` no longer derives `Clone`, and `DocumentKey` keeps it by design,
documented on the type: each copy wipes itself on drop.

---

## ADDENDUM 2026-07-08 (2) -- SURFACED WHILE IMPLEMENTING WAVE R6

Two issues in the account-setup path noticed during the N3 work. Neither was in
the R6 scope; both are recorded here for a follow-up wave.

### N4 (HIGH) -- Passphrase transmitted in cleartext to the server (zero-knowledge breach)

**Crate:** `ambre-server` / `ambre-sync`
**Files:** `sync/auth.rs` (`handle_setup_account`), `routes/documents.rs`
(`SetupAccountRequest`)

`handle_setup_account` puts the raw passphrase in the setup request body
(`passphrase: String::from_utf8_lossy(&passphrase).into_owned()`), and the server
`SetupAccountRequest` still has a `passphrase: String` field (marked
`#[allow(dead_code)]`, never used). The passphrase is client-only secret material:
the master key is derived from it *client-side* (Argon2) and only the *wrapped*
seed plus salt should ever reach the server. Shipping the plaintext passphrase
over the wire -- where it lands in request logs, proxy buffers, and the server
process -- directly violates the Zero-Knowledge pillar ("the server only knows
routing metadata; no content, no encryption keys"). The field being unused
server-side does not help: it is on the wire and in memory.

**Impact.** A compromised or curious server (or any log/proxy in the path) learns
the passphrase, from which the master seed and every DocumentKey derive. This is
the strongest secret in the system leaking to the exact party the E2EE model
assumes is untrusted.

**Fix:** drop the `passphrase` field from both the sync `http::SetupAccountRequest`
and the server `SetupAccountRequest`; the server never needs it (it only stores
`master_key_enc` + `master_key_salt`). Add a regression assert that the setup
request body carries no passphrase field.

### N5 (LOW) -- No uniqueness on `users.email` weakens the N3 identity

**Crate:** `ambre-server`
**Files:** `migrations/20261024000000_initial_schema.sql` (`email TEXT NOT NULL`),
`db/users.rs` (`get_user_by_email`)

N3 now records the client-supplied `email`, but the column has no `UNIQUE`
constraint, so multiple accounts can share one email. That (a) leaves the original
account-creation spam vector from N3 only partially closed, and (b) makes
`get_user_by_email` (used by `invite_to_document`) ambiguous -- it resolves the
first match, so which identity an invite targets is not well-defined once
duplicates exist. Deferred from R6 because a `UNIQUE` constraint is a migration
that must also account for the OIDC placeholder rows.

**Fix:** add a `UNIQUE` constraint (or partial unique index) on `email`, decide
the OIDC-vs-setup collision policy, and map the insert conflict to `409 Conflict`
rather than the current `500` (a unique violation currently surfaces as
`AppError::Database`).

---

## ADDENDUM 2026-11-14 -- PRE-AUDIT OF THE CRYPTOGRAPHIC DESIGN

This addendum answers the A security pre-audit of the cryptographic design Story (`ambre-roadmap.md` E7). Four AI
reviewers worked apart, each on one scope, with a neutral brief: the files and specs to read and the
threat model, never the conclusions of another. The author of this addendum checked each finding
against the code, kept the ones with a realistic attacker path, and fixed or answered each one.

**Scopes.** The key hierarchy, its rotations and revocation. The sealed envelopes, the AEAD, the
nonces and what is signed. The call chunks, the sender keys, the anti-replay window and the
admission. The zero-knowledge boundary of the relay: its routes, its logs and its stores. Each scope
covers the design and the code that carries it: `ambre-crypto`, `ambre-sync`, `ambre-protocol`, the
relay and the SFU.

**Threats.** A malicious relay. A malicious SFU. A member removed, who kept every key and
ciphertext. A thief of the database, the ephemeral store or the object store. A current member of a
lower role.

**Known defects, out of scope.** Three Stories of E7 hold them, and this pass fixed none. The first
two landed on `main` while this pass ran:
`A rotation wraps the key for the exchange keys the log signs`,
`An open accepts no current key the relay chose`, and
`A member sees that the relay withheld a log entry`. The second one closed finding `N13`. Findings
`N14` to `N16` were each an `unimplemented` row bound to the follow-up Story of E7 their subject
names. `A first open trusts no wrapper the relay names` then closed `N15`.

### Numbered findings

`Held by` names the row that goes red if the fix is reverted, or the row of the Story that owes it.

| ID | Severity | Finding | Answer | Held by |
| :-- | :-- | :-- | :-- | :-- |
| N6 | high | A replay batch delivered each update under the document the relay named, while the update opened under the document its sealed metadata named. A relay moved the plaintext of one document into another, across a membership boundary | fixed -- the Sync worker drops an update filed in the replay of another document | `SYNC-203` |
| N7 | high | The creator of a document regained an owner row by posting the creation again, after a co-owner removed it, after it left, or on a direct message that has no owner. It also read the metadata as a non-member | fixed -- the retry registers the creator only while no member row names the document, and a creator without a row reads not-found | `SRV-268`, `SRV-005` |
| N8 | medium | An editor re-invited another editor through the invite upsert: it demoted that editor and replaced its wrapped key and its wrapper with ones it chose | fixed -- only an owner re-invites a member who holds a row | `SRV-266` |
| N9 | medium | A member removed while connected kept every broadcast of the document while it sent nothing: the re-check ran on inbound frames alone. A public channel owes no rotation, so its new messages stayed readable | fixed -- the socket re-reads each watched membership once per thirty seconds and drops the routes of a removal | `SRV-043` |
| N10 | medium | The ephemeral store named each refresh token and each sign-in state in the clear in its key. A copy of the store signed in as every user for thirty days | fixed -- the store key is the SHA-256 of the value | `SRV-267` |
| N11 | high | Every participant holds the key of every sender, and neither the SFU nor the receiver tied a chunk to the connection that sent it. Any participant spoke as another, or silenced another by moving its replay window. `CALL-025` read `verified` on a test whose forger held no key | fixed -- the relay signs the sender index into the admission, and the SFU forwards no chunk under another index. `CALL-025` now states what its test holds | `CALL-116`, `CALL-025` |
| N12 | low | An admission seated a second connection once the first one left, for the rest of its sixty seconds | fixed -- the SFU keeps each seated pseudonym until its admission expires | `CALL-117` |
| N13 | high | A wrap binds no document and no key, and the client installed a rotated key with no link to the key it held. A relay served a genuine older wrap and rolled a member back to the key before a revoke, or to the key of another document | closed -- the An open accepts no current key the relay chose Story (`ambre-roadmap.md` E7) refuses a key a writer replaced, or one no writer of this log named, at open and on a rotation | `SYNC-202`, `SYNC-200` |
| N14 | medium | The membership log keeps every key that ever held a writing role as a link signer, and the key check reads that set. A revoked editor, with the relay, signed a link from a key of its own to a key the member named, and the check accepted it | fixed -- a link signs the log head its rotator read, and its signer must write at that head. A link at a head before a revoke vouches for no key once a writer named another key past it. A departure names no key, and a key in force before it stands only while the debt is owed. A writer records that debt from its own log | `SYNC-204`, `SYNC-221`, `SYNC-222`, `SYNC-223` |
| N15 | high | The first entry of the log comes inside the wrap the relay chooses, and no device pins it. A relay rooted a log of its own on a wrap of its own, and every row of it read verified | closed -- the A first open trusts no wrapper the relay names Story (`ambre-roadmap.md` E7) refuses a log the inviter the link pins signed no entry of, and an open with no link unless this client wrapped its copy or signed an entry. The A Murmure open roots its log on a pin Story (`ambre-roadmap.md` Murmure) closes the same finding for Murmure: a Murmure open refuses a log no root of the session signed | `SYNC-205`, `SYNC-210`, `SYNC-235` |
| N16 | high | A channel key in the space catalogue is sealed under the space key and signed by nobody. A viewer of the space, with a relay that forwards its delta, handed each self-joiner a key and an anchor it chose | closed -- the A channel document accepts no current key the relay chose Story (`ambre-roadmap.md` E7) checks the writers of the space document against its log, so a viewer's catalogue entry reaches no client, and refuses a channel log that no writer of the space created | `MURMURE-635` |

### Answered in writing

These findings state no rule this pass could hold. Each carries an answer, and the ones that owe
work name the Story the report to the parent proposes. Their identifiers are local to this table.

| Ref | Severity | Finding | Answer |
| :-- | :-- | :-- | :-- |
| P1 | medium | The client applies a delta or a snapshot from the holder of any key of the document, current or retired, whatever the role of the signer. A viewer, or a removed member, writes through a relay that forwards it | closed -- the A client refuses a delta whose signer holds no writing role Story (`ambre-roadmap.md` E7) refuses a signer the log never granted a writing role (`SYNC-211`). A writer revoked later stays a signer, by decision of `ambre-spec-membership.md` |
| P2 | medium | No floor on the strength of the passphrase that alone guards the master seed against a thief of the account table | closed at account setup -- `ZK-035` refuses a seal under 48 bits. The change and the recovery are owed by the A passphrase change or a recovery takes the strength floor Story (`ambre-roadmap.md` E7) |
| P3 | medium | `POST /keys/master/rotate` replaces the wrapped master key on a session alone, so a stolen session locks its owner out of the seed | closed -- each re-seal carries a proof the account's signing key signs over the seal it replaces (`SRV-278`, `SRV-279`, `SYNC-254`) |
| P4 | low | The user search answers the address of any account for a two-character fragment, and the account setup route needs no session | fixed -- a search by address answers the account whose whole address it is, and no other (`SRV-280`). The setup route stays open by design: it confirms one guessed address per request, as the search does |
| P5 | low | A signed join carries an instant no client checks, so a relay replays an old join to a member who joined later, who wraps its key to it. The participant is a member of the channel, and could join openly | answered -- `CALL-124`: a client refuses a later join older than its own. A join of an earlier session in the roster is the limit of `ambre-spec-call-media.md section 6` |
| P6 | low | A snapshot opens without a check that its sealed identifier is the one asked for, so a relay swaps two snapshots of one document | closed -- the An opened snapshot is the one asked for Story (`ambre-roadmap.md`) seals the id the relay mints with the upload link, and an open refuses any other (`SYNC-257`, `SYNC-258`) |
| P7 | low | A receiver keeps the older epochs of a sender open until eight openers pass, so a chunk under an older key still opens after the switch | fixed -- once a chunk of a sender opens under a newer epoch, the receiver drops every older key of that sender (`CALL-125`) |
| P8 | low | The escrow column is one per user, not per organization, and an owner adds any user to its organization | answered -- nothing writes the column until `SRV-067` lands, and that Story keys it by organization and member |
| P9 | info | A TURN credential lives twelve hours, past a removal | answered -- by design, `ambre-spec-call-media.md section 3`: it grants bandwidth and no key |
| P10 | info | A snapshot compresses the content of every author in one frame, and the relay sees its size | answered -- an oracle needs a party that writes and cannot read. None exists today; a form inbox would make one |
| P11 | info | The document key serves as the AEAD key and as the HMAC key of the commitment tag, with no derived subkey | answered -- no attack is known on the pair. A subkey per purpose is the cleaner form, owed to the external `Call crypto review` |

### Checked and found sound

Each reviewer listed what it checked and found to hold. The nonces: a fresh 192-bit draw per seal,
and a counter per chunk key issued once. The strict Ed25519 verifiers, and the length-framed signed
transcripts. The commitment tag, which commits the key. The domains of the seed derivation. The key
links, which walk back and never forward. The chunk keys, one per sender, track and generation. The
replay window, marked only after the tag verifies. The admission, under a key of its own. No
plaintext and no unwrapped key reaches a log line, a table or a key of the relay.

## ADDENDUM 2026-11-16 -- SECOND PASS OF THE CRYPTOGRAPHIC PRE-AUDIT

This addendum continues the A security pre-audit of the cryptographic design Story
(`ambre-roadmap.md` E7). Since the first pass, the inviter pin, the withheld-entry check and the
signer refusal landed on `main`. Three AI reviewers audited the tree of that day apart, with the
threats of the first pass and the list of its findings, so that none was reported twice.

**Scopes.** The key hierarchy, the membership log and the checks at open and on a rotation. The
sealed envelopes, the zero-knowledge boundary and what the client accepts from the relay. The
call chunks, the sender keys, the admission and the SFU.

**Out of scope.** Six sessions in progress own known defects, and this pass answers them by name:
`channel-meta-key` (`N16`), `wrapper-exchange-key` (`N14`), `withheld-stops-sealing`,
`log-head-signed`, `key-relay-fault` and `passphrase-change-floor` (`P2`).

### Numbered findings

| ID | Severity | Finding | Answer | Held by |
| :-- | :-- | :-- | :-- | :-- |
| N17 | medium | The walk let an editor re-invite a present member, and let a self-join name a present member, under keys the entry chose. A relay stored the editor's entry, so a later rotation wrapped for the editor's key, and a call admitted the editor as that member | fixed -- only an owner's invite replaces the keys of a present member | `SYNC-217` |
| N18 | high | A members refresh replaced the writers the open walked with whatever the refresh walked. A relay dropped the caller's row, the walk ran unanchored, and every signer passed. A relay that served a wrap and a log of its own re-rooted the writers | fixed -- a refresh that drops a writer the held log granted keeps the writers and the space word | `SYNC-214` |
| N19 | high | The directory that names the author of a delta and a cursor read the relay's rows alone. A relay served Alice's row with Mallory's key, and Mallory's message passed the Murmure author check as Alice's | fixed -- a row whose key no valid entry bound to that member is left out | `SYNC-215` |
| N20 | high | A rotation installed any key the members answer wrapped. A relay withheld the link past a retired key, served the retired wrap again, and rolled an open document back to the key a revoke retired | fixed -- a rotation back to a key the cache or the chain reaches keeps the current key. A first open refuses the same roll back (`N23`) | `SYNC-213`, `SYNC-226` |
| N21 | low | A presence sample moved its author's replay counter before its signature was checked. A relay forged one sample at the highest counter and silenced that author's cursor for the session | fixed -- a sample moves the counter only once it opens | `SYNC-216` |
| N22 | low | The switch to a fresh sender key waited on one timer. A wall clock that stepped back left the switch not due at that tick, and no tick followed, so a removed participant kept the key | fixed -- a tick that finds the switch pending asks another | `CALL-118` |
| N23 | high | At a first open, the key check read every key a writer ever named. A relay served a member's genuine wrap of a key before a revoke, with no link past it, and the open installed it. The member then sealed under a key the removed member holds | fixed -- the links a writer signed must walk back from the opened key to the latest key a valid entry names. A relay that withholds the revoke entry itself is the case of the log head a member announces | `SYNC-226` |
| N24 | high | A self-join of a public channel's log was checked against the key it names itself. The relay alone appended one for a member of its choice under a key of its own: the call admitted it and a channel rotation wrapped the key for it. Reported as `P13` | fixed -- a self-join counts only for a present member of the space, under the keys the space log binds | `SYNC-227` |
| N25 | high | A member opened SFU seats it never announced, then closed its relay socket. The SFU kept each seat with no renewal, so the seats filled the room past a removal, and headerless audio packets held the three loudest-speaker slots (`P15` below) | fixed -- a seat lives while the relay renews its admission for a member of the channel, a member holds one seat of a room, and only a packet that names its seat counts as speech | `CALL-119` to `CALL-123` |
| N26 | medium | The relay signed an upload link for any address once the role check passed. A writer put a blob of its own over the blob of another member, and every reader then failed the address check (was `P16`) | fixed -- the relay signs no upload link for an address the document holds. A link signed before the first confirmation stays valid for its lifetime: Garage 2.3 ignores a conditional put, so that window stays open | `SRV-271` |

### Answered in writing

Each finding below owes a Story the report to the parent proposes, or carries its answer. A
finding that owes a Story takes a numbered identifier and its contract row once that Story
reaches the roadmap. The identifiers are local to this table.

| Ref | Severity | Finding | Answer |
| :-- | :-- | :-- | :-- |
| P12 | info | The signature of a registry entry covers the document, the PeerID and the nonce, not the member it names. A snapshot extra signed by its author passes whatever member it names | answered -- Murmure refuses a snapshot that carries extras, and nothing reads the member of an entry today. The first consumer binds the member to the author's key in the log, as `SYNC-215` does |
| P14 | high | At a first open, the key check reads every key a writer ever named. A relay served a member's genuine wrap of a key before a revoke, with no link past it, and the open installed it | numbered `N23` once its Story reached the roadmap |
| P15 | high | A member opened SFU seats it never announced, then closed its relay socket. The SFU kept each seat with no renewal, so the seats filled the room past a removal, and headerless audio packets held the three loudest-speaker slots | numbered `N25` once its Story reached the roadmap |
| P16 | medium | A writer asked an upload address for the hash of a blob another member stored, and overwrote it. Every reader then failed the address check, and the erase route refused the writer too late | numbered `N26`, and fixed |
| P17 | low | The SFU reads the sender index of a VP8 frame from its first packet alone, and passes a packet of any codec it cannot read. A participant who held another sender's key forged a video frame as theirs | owed by a proposed Story: the SFU offers Opus and VP8 alone, and drops a frame start that names no index. No view labels a video tile with its sender today |

### Checked and found sound

The order of the open: the signature, then the commitment tag, then the cipher. The additional
data of each frame, which binds the document and the key. The domains of the standalone seals.
The bounded decompression. The KEK derivation, which binds both public keys and refuses a
low-order point. The canonical entry encoding and its digest chain. The chunk seal, its nonce and
its counters. The replay window, the ratchet bound and the sender keys drawn fresh at each leave.
The join signature, which covers the index and the exchange key.

## ADDENDUM 2026-11-17 -- THIRD PASS OF THE CRYPTOGRAPHIC PRE-AUDIT

This addendum continues the A security pre-audit of the cryptographic design Story
(`ambre-roadmap.md` MVP). Three AI reviewers audited the tree of the batch of that day apart. Each
had the threats of the first pass, the findings of the two passes and the defects the roadmap
already holds. The author of this addendum checked each finding against the code before keeping it.

**Scopes.** The key hierarchy, its rotations and the space and channel keys. The sealed envelopes
and the zero-knowledge boundary. The call chunks, the sender keys, the joins and the call log.

**Out of scope.** A session in progress gives the public channels of a space one key
(`space-one-key`). This pass fixes nothing on that key, and names each finding on it.

### Numbered findings

| ID | Severity | Finding | Answer | Held by |
| :-- | :-- | :-- | :-- | :-- |
| N27 | high | The call admitted its joiners against the log of the `meta` that a members answer served, walked from the anchor of the client's own copy alone. No check of an open ran on it. The relay wrapped a key of its own to the client, rooted a log of its own on it, and seated itself: every participant wrapped its sender key to the relay | fixed -- the log of a call counts only once it stands on the anchor, the pinned inviter or a root of the session, and a wrapper the log verifies. A first log with no root ends the join | `CALL-134` |
| N28 | medium | A join marked its exchange key as seen before the membership check. Any member opened a second seat, then announced a join of a stranger under the exchange key of a later joiner. Each later participant then refused the genuine join as a replay | fixed -- only an admitted join marks its key, under the identity that signed it | `CALL-133` |

### Answered in writing

Each finding below owes a Story the report to the parent proposes, or carries its answer. A
finding that owes a Story takes a numbered identifier and its contract row once that Story
reaches the roadmap. The identifiers are local to this table.

| Ref | Severity | Finding | Answer |
| :-- | :-- | :-- | :-- |
| P18 | high | `space-one-key`. The space word grants a link of a public channel to every key the space log ever granted a writing role, at every head. A writer the space removed, with the relay, signs a link to a key of its own past the latest head, and the rotation installs it. `SYNC-204` holds for a document alone | owed to the session that gives a space one key for its public channels, or to a proposed Story: the space word grants the writers present at each head, and a member the space removed closes its stretch |
| P19 | medium | `space-one-key`. A space admin re-keys a public channel under a rule with no space word, so the walk stops at the first self-join. No member past it gets a copy, and each loses the channel for good | owed to the same session or Story: the re-key walks under the word of the space it holds |
| P20 | medium | `space-one-key`. The media address is keyed by the document key alone. Once two channels share one key, the relay sees the same file stored in both | owed to the same session: the address binds the document before two documents share a key. `SYNC-050` holds only while each document has its own key |
| P21 | high | A cold open reads whether a snapshot was confirmed by another member from the relay's word alone, and adopts a confirmed snapshot whole. The relay and one writer attribute forged operations to another member, as section 5 of `ambre-spec-op-authorship.md` rejects | owed to a proposed Story: a confirmer signs the document, the snapshot, the horizon and the author, and an open trusts a snapshot whole only under such a signature of a member other than its author |
| P22 | low | The presence counter is keyed by the user's signing key, and a sender starts it again at each open. A reopened author is silent to a peer that stayed open, and a fresh peer accepts a replayed old sample | owed to a proposed Story: the guard reads a session nonce the sample carries. `ambre-spec-sealed-envelope.md` section 2 now states the reset |
| P23 | low | A light subscription counts the summary and rings the call cue of any update that opens, with no writer check, and deduplicates on the relay's number alone. A viewer and the relay inflate the mentions of an evicted channel | owed to a proposed Story: a light subscription keeps the writers its last walk granted, and a channel never opened walks its log first |
| P24 | medium | The relay names the `meta` of the channel of a call, and the first read of its log may be a genuine answer from before a revoke. A relay names the `meta` of another channel, or rolls the log back, and a member of its choice joins | owed to a proposed Story: the Logic thread hands the `meta` its sealed channel state names with the join, as it does for a roster ask. The roll back is the case of the log head a member announces (`N23`) |
| P25 | info | The update, the snapshot and the ephemeral seal the same shape of additional data, with no domain of their own | answered -- moving one into another needs a canonical parse of the other over the same bytes, and the payload then fails to decode. A domain per kind is owed at the next layout change |

### Checked and found sound

The seed domains, the recovery checksum and the signed re-seal. The key links, which bind the
document, the previous key and the head. The order of the open, and the strict verifiers. The
chunk header, its counters and the replay window, marked after the tag. The wraps of a call,
which bind the call, both indexes and both join keys. The handoff port, the video choice and the
ring cue. The standalone seals, which each bind their document. No plaintext and no key reaches a
key of the ephemeral store or of the object store.

---

## REMEDIATION ROADMAP

Security-first ordering, grouped into waves. Same `[ ]`/`[x]` convention as
`ambre-roadmap.md` -- a wave is `[x]` only when every story under it is `[x]`.
Each story carries its finding ID, the file to touch, and the concrete action.

### Wave R0 -- Authorization holes (do first; small diffs, real bypasses) [x]

- [x] **C4** -- `db::get_snapshot` takes `document_id` and filters
      `WHERE id = $1 AND document_id = $2`; `snapshot_download_url` passes the
      path `doc_id`, returns 404 on mismatch. (`server/db.rs`, `routes/sync.rs`)
      Regression guard added in the `create_snapshot` roundtrip test.
- [x] **C4-audit** -- swept: `get_snapshot_metadata` (via `get_latest_snapshot`)
      and `get_deltas` (via `get_updates_after`) already scope by `doc_id`;
      `get_snapshot` was the only bare-id fetch, now fixed.
      (`server/routes/sync.rs`, `routes/media.rs`)
- [x] **C5** -- `s3::is_valid_content_hash` enforces `^[0-9a-f]{64}$`; all three
      media handlers reject a malformed hash with `BadRequest` before the key
      builder runs. Unit-tested (`s3::tests::content_hash_validation`).
      (`server/routes/media.rs`, `s3.rs`)
- [x] **C5-hardening** -- WON'T DO (by design). Server-side BLAKE2b-256 re-hashing
      of uploaded bytes would force the server to GET and stream the blob body,
      breaking the Blind Relay pillar (server never touches blob content) and the
      direct client->S3 presigned upload path. The traversal threat this story
      guarded against is already fully closed by the `^[0-9a-f]{64}$` validation
      (C5); the residual "hash does not match bytes" risk is inert here because
      the payload is E2EE and its real integrity is enforced downstream at open
      time by AEAD + HMAC commitment + Ed25519 signature. Trusting the client
      hash only as an S3 key (never as an integrity proof) is acceptable given
      those guarantees. (`server/routes/media.rs`)

### Wave R1 -- Peer-triggered DoS & live-session authz [x]

- [x] **H9** -- `decompress_zstd_bounded` streams the zstd body through a
      `Read::take(ceiling + 1)` cap and rejects with `DecompressTooLarge` before
      the full output allocates. Ceiling = `body.len() * 200` clamped to
      `[1 MiB, 64 MiB]`. Guarded by `decompress_bomb_rejected` /
      `decompress_within_ceiling_ok`. (`sync/pipeline.rs`)
- [x] **H8** -- `handle_update` re-consults `document_members` on a 30 s TTL
      (`MEMBERSHIP_RECHECK_TTL`) via `doc_access_verified_at`; on a failed
      re-check it drops the subscription (local + Redis session) and returns 403,
      so a mid-session revoke stops write acceptance within the TTL instead of at
      disconnect. (`server/ws/handler.rs`)
- [x] **M16** -- `params.limit.unwrap_or(100).clamp(1, 1000)` in `get_deltas`.
      (`server/routes/sync.rs`)

### Wave R2 -- Make the E2EE story honest (fail-closed + secret hygiene) [x]

- [x] **H3** -- the open-document task now sends `SyncEvent::OpenDocumentFailed`
      (surfaced as `LogicEvent::AuthError`) when the wrapped DocumentKey cannot be
      unwrapped for a server-side doc; the `unwrap_or_else(|_| generate())` random
      fallback is gone. Local-only (404) opens still generate a fresh key as
      before. (`sync/lib.rs`)
- [x] **H4** -- `handle_change_passphrase` / `handle_recover_account` take the new
      `SecretBytes` / `SecretString` types, which zeroize on drop; the
      `let _ = (...)` bind-and-leak is replaced by an explicit `drop(...)` with a
      comment. (`sync/lib.rs`)
- [x] **H5** -- new `SecretBytes` (`Zeroizing<Vec<u8>>`) / `SecretString`
      (`Zeroizing<String>`) IPC types carry the secret fields of
      `Unlock`/`SetupAccount`/`ChangePassphrase`/`RecoverAccount`: redacted
      `Debug`, wiped on drop everywhere along the relay, exposed only via `Deref`.
      The `bus/rest.rs` "zeroized after use" comment is now true. (`bus/rest.rs`,
      `sync/lib.rs`)
- [x] **H2** -- `get_document` returns 404 when the caller is neither a member nor
      the owner, so `owner_id`/`doc_type`/existence no longer leak. Regression
      guard: `get_document_hidden_from_non_member`. (`server/routes/documents.rs`)
- [x] **C1** -- the S3 flusher error paths no longer log `blob_str` (which carries
      `update_b64` ciphertext); they log only `key`, `blob_len`, and once parsed
      `s3_key` / `seq`. (`server/s3.rs`)

### Wave R3 -- Crypto hardening (cheap, while the surface is small) [x]

- [x] **M13** -- `signature::verify` and the server's `verify_update_signature`
      now use `verify_strict` (canonical `S`, no small-order `A`), closing
      signature malleability on the author-attribution primitive. The `Verifier`
      trait import is dropped on both sides (`verify_strict` is inherent).
      (`crypto/signature.rs`, `server/ws/handler/message.rs`)
- [x] **M14** -- the KEK is now derived via HKDF-BLAKE2b (`SimpleHkdf`, required
      by BLAKE2b's variable-output core) with
      `info = KEK_DOMAIN || sender_pk || recipient_pk`; wrap/unwrap feed the
      identity pair in the matching sender/recipient order. `shared_secret`
      rejects a non-contributory (low-order) peer key with
      `CryptoError::WeakPublicKey`. Domain bumped to `ambre-kek-v2` (not
      wire-compatible with v1). Regression guards: `weak_peer_public_key_rejected`,
      `kek_binds_peer_identity`. (`crypto/ecdh.rs`, `crypto/error.rs`)
- [x] **crypto-hygiene** -- `aead::seal` returns `Result<_, CryptoError>`
      (`EncryptionFailed`) instead of `.expect`; the error propagates through
      `envelope::seal` and the `pipeline::seal_*` chain. `MasterSeed` no longer
      derives `Clone`; its raw `as_bytes` accessor is replaced by a named
      `recovery_bytes()` (recovery escape hatch) plus a `wrap_for_storage()`
      helper that seals the seed inside the crate, so the root seed never leaves
      `ambre-crypto` on the storage path. `DocumentKey` keeps `Clone`
      deliberately: it is moved into `spawn_local` closures while retained in Sync
      state, so `Rc`-izing it is out of scope and buys nothing (each copy is still
      `ZeroizeOnDrop`). Own-key seal/wrap call sites use `.expect` (infallible for
      our own valid key + bounded plaintext), matching the existing
      `.expect("CSPRNG")` style; peer-facing `wrap_key_b64` propagates the error
      (invite fails, rotation skips the member). (`crypto/keys.rs`, `crypto/aead.rs`,
      `sync/pipeline.rs`, `sync/sharing.rs`, `sync/auth.rs`)
- [x] **argon2-review** -- confirmed against `Cargo.lock`: `argon2` 0.5.3, no
      relevant RUSTSEC advisory. `LANES` kept at 1 (documented): the KDF runs in a
      single-threaded WASM worker, so `p > 1` only adds sequential work with no
      parallel speedup, and memory-hardness is governed by `MEM_KIB`, not the lane
      count. Params stay versioned so the lane count can rise if a threaded target
      lands. (`crypto/passphrase.rs`)

### Wave R4 -- Robustness / operational [x]

- [x] **H1** -- `handle_update` now logs `append_pending_delta` / `doc_broadcast`
      failures at ERROR (instead of `let _ =`); a failed pending-push (the only
      path a sequenced delta takes to S3) is dead-lettered via the new
      `redis_ops::push_dead_letter` onto `doc:{id}:deadletter` for operator
      replay. (`server/ws/handler/message.rs`, `redis_ops.rs`)
- [x] **M6** -- fixed-window `INCR`/`EXPIRE` replaced by an atomic sliding-window
      sorted-set Lua script (`SLIDING_WINDOW_LUA`: prune/add/count/PEXPIRE in one
      round trip), killing the 2x boundary burst and the TTL-less-key window.
      The auth tier now fails **closed** (503) on a Redis outage
      (`RateLimitCategory::fail_closed`); API/WS stay fail-open. Guard:
      `only_auth_tier_fails_closed`. (`server/middleware/rate_limit.rs`)
- [x] **M7** -- OIDC init failure is no longer swallowed by `.ok().unwrap_or_default()`:
      when credentials are configured a discovery failure logs at ERROR and
      fail-fasts startup; when unconfigured it warns and disables provider login.
      (`server/main.rs`)
- [x] **M9** -- every Sync HTTP call attaches an `AbortSignal::timeout(30 s)`
      (`timeout_signal`), so a hung connection aborts instead of stalling the
      Sync worker future forever. `web-sys` gains the `AbortSignal` feature.
      (`sync/http.rs`, root `Cargo.toml`)
- [x] **M10** -- `SyncEvent::WsError` now tears down the dead socket
      (`handle_ws_disconnected`) and takes the same `schedule_reconnect()` backoff
      path as `WsDisconnected`, instead of only logging; an `onerror` without a
      following `onclose` no longer stalls reconnection. (`sync/websocket.rs`)
- [x] **M15** -- the server `TraceLayer` span records only the URI *path*, never
      the query string (the WS `?token=` JWT no longer reaches the access log);
      the reverse-proxy log-stripping requirement and a future one-time-ticket
      option are documented in `ambre-spec-session.md`, `SRV-031`, and on `WsQuery`.
      (`server/main.rs`, `ws/handler/mod.rs`, `ambre-spec-session.md`)
- [x] **M17** -- CORS is config-driven and default-deny: `CORS_ALLOWED_ORIGINS`
      (comma-separated) drives an explicit allow-list; `allow_origin(Any)` is only
      reachable behind `DEV_MODE` (logged at startup). Parsing guard:
      `cors_tests`. (`server/main.rs`)

### Wave R5 -- Maintainability debt (no security impact) [x]

- [x] **trame-formula** -- range registration in `formula/dag.rs` is already
      two-level (O(ranges), not O(range_size)); the residual O(range_size) hazard
      was `eval_range` materializing one `CellValue` per member cell, so a peer's
      `=SUM(A1:ZZ100000)` could OOM a collaborator's Logic worker. Capped at
      `MAX_RANGE_CELLS` (1M) -> `#REF!` past the ceiling
      (`eval_range_over_cell_cap_yields_ref_error`). `flush_delta` no longer
      `.ok()`-swallows a failed Loro export: it logs at ERROR and RETAINS the
      export cursor so the delta is retried instead of silently lost (which,
      with remote apply live, would mean permanent divergence).
      (`trame/src/logic/sync.rs`, `formula/vm.rs`)
- [x] **trame-dedup** -- the four `col_to_alpha`/`col_label`/`col_letter` copies
      and the two `alpha_to_col` copies now delegate to a single crate-level
      `columns` module; the five `lines() -> GlyphRun -> draw_glyphs` loops
      collapse into `render::text::draw_layout_runs` (+ a `_decorated` variant
      for the underline/strikethrough cell paths). Guards added: `LEFT`/`RIGHT`
      reject a negative/non-finite length with `#NUM!` (`str_count_arg`, no more
      `as usize` wrap), `Pow` surfaces NaN/inf as `#NUM!`. (`trame/src/columns.rs`,
      `formula/vm.rs`, `formula/functions/builtins_a.rs`, `render/**`, `ui/**`)
- [x] **canvas-eviction** -- `TextCache` evicts by LRU (`last_used` tick) down to
      a 75% low-water mark instead of `clear()`ing the whole map (M4). The glyph
      atlas keeps a per-glyph retained bitmap + `last_used` frame and, on
      overflow, defers to a next-frame `compact()` that repacks the last
      `KEEP_FRAMES` working set from the retained bitmaps (no re-rasterization,
      no blank flash) instead of the old full-clear (M3). Deferred so it never
      rewrites UVs already baked into the current frame's vertex buffer.
      (`canvas/text_cache.rs`, `canvas/render/atlas.rs`)
- [x] **canvas-gpu** -- `webglcontextlost` (with the required `preventDefault`)
      and `webglcontextrestored` listeners are wired on the canvas;
      restore reconfigures the surface + offscreens and forces a full repaint
      (`FrameHost::notify_context_restored`). A `Lost`/`Outdated`
      `get_current_texture` now reconfigures and skips the frame instead of just
      logging. The DPI watcher is fixed: the old `match_media("(resolution)")`
      never fired (no threshold); it now queries `(resolution: Ndppx)` and
      re-arms on each change (M1/M2). (`app/host/run.rs`, `app/host/frame.rs`)
- [x] **canvas-stroke** -- `stroke_rect` was already a non-overlapping
      picture-frame (two full-width horizontals owning the corners + two inset
      verticals), so the M5 translucent double-blend was closed by the refactor;
      documented, and guarded the degenerate rect-shorter-than-stroke case that
      would otherwise emit an inverted vertical quad. (`canvas/render/surface.rs`)
- [x] **server-hygiene** -- the flusher `ltrim` desync is fixed: it now trims by a
      `processed` count that advances on BOTH successful uploads and dead-lettered
      poison entries and stops only on the first TRANSIENT failure, so a poison
      entry ahead of a good one no longer re-uploads the good one every cycle and
      a trailing poison no longer retries forever (dead-lettered to
      `{prefix}:deadletter`). The request-path `postcard::to_allocvec(update)
      .expect(...)` is replaced by log-and-return (the remaining non-test
      `.expect()`s are startup/config invariants). Pruning `doc:{id}:clocks` is a
      WON'T DO (documented on the Lua script): a TTL would lock a returning
      author out under the strict-clock invariant, per-author prune has no
      `user_id -> signing_key` mapping to target, and either would reopen the
      replay window the hash exists to close; growth is already membership-bound.
      (`server/s3.rs`, `ws/handler/message.rs`, `redis_ops.rs`)

### Wave R6 -- Organizations / key-escrow subsystem (new surface, 2026-07-08) [x]

- [x] **N1** -- recovery is now `owner`-only (was `owner`/`admin`) and
      `create_org` rejects `shamir_threshold > 1` (plus a `1 <= threshold <= total`
      bounds check) via `validate_shamir_params`, so the schema never advertises a
      k-of-n quorum the server cannot enforce while threshold reconstruction is
      unimplemented. `recover_master_key`'s doc comment now states the
      single-authority contract loudly. Guards: `shamir_threshold_1_accepted`,
      `shamir_threshold_above_1_rejected_until_reconstruction_lands`,
      `shamir_out_of_range_rejected`. (`server/routes/orgs.rs`)
- [x] **N2** -- `invite_to_document` ranks roles (`doc_role_rank`: owner > editor >
      viewer) and clamps the granted role to `<= caller.role`, so only an `owner`
      may grant `owner`; an unknown role is rejected. Guards: `doc_role_rank_ordering`,
      `editor_cannot_grant_owner`, `owner_can_grant_any_valid_role`.
      (`server/routes/share.rs`)
- [x] **N3** -- `SetupAccountRequest` now carries client-supplied `email` /
      `display_name` (plumbed through `RestCommand::SetupAccount`, the sync
      `handle_setup_account` -> `http::SetupAccountRequest`); the hardcoded
      `temp@ambre.app` / `"User"` is gone. Server-side `validate_identity` bounds
      the email (`3..=254`, must contain a non-terminal `@`) and display name
      (non-empty, <= 128 chars). Guards: `validate_identity_*`.
      (`server/routes/documents.rs`, `sync/auth.rs`, `bus/rest.rs`, `sync/http.rs`)
- [x] **R6-residuals**:
      - **Signature transcript length-prefixed** -- `signature::message` now frames
        each field as `len(u32 LE) || bytes` (nonce, ciphertext, aad,
        commitment_tag), so the ciphertext|aad boundary is unambiguous. The
        server's independent `verify_update_signature` mirrors the exact framing
        (`push_framed`, must stay byte-identical, documented on both sides). Guards:
        `ciphertext_aad_boundary_is_unambiguous`, `sign_then_verify_roundtrips`.
        (`crypto/signature.rs`, `server/ws/handler/message.rs`)
      - **Sync `AuthError` sanitized** -- the unlock error paths no longer leak
        crypto detail (algorithm names, key/salt lengths, decrypt specifics) into
        the user-facing message; they emit two generic strings
        (`account data is corrupted` / `incorrect passphrase or corrupted account
        data`) and log the concrete cause at WARN. (`sync/auth.rs`)
      - **Ephemeral remote-counter map bounded** -- `accept_ephemeral_counter` caps
        the per-document map at `MAX_EPHEMERAL_REMOTE_AUTHORS` (512) and evicts the
        least-active (lowest-counter) author at the cap, so a peer minting
        arbitrary signing keys can no longer grow it without bound. Guards:
        `monotonic_counter_accepted_replay_dropped`, `author_map_is_bounded`.
        (`sync/lib.rs`, `sync/websocket.rs`)

### Wave R7 -- Account-setup path (surfaced during R6) [x]

- [x] **N4** -- the passphrase no longer ships to the server: the `passphrase`
      field is removed from both the sync `http::SetupAccountRequest` and the
      server `SetupAccountRequest` (the server only receives `master_key_enc` +
      `master_key_salt`), restoring the Zero-Knowledge invariant; the invariant
      is documented on both structs. The sync worker also stops logging the
      passphrase length on the Unlock/SetupAccount paths (secret metadata in
      the console). (`sync/auth.rs`, `sync/http.rs`, `server/routes/documents.rs`)
- [x] **N5** -- `users.email` is now globally unique, case-insensitively
      (`users_email_lower_unique` on `lower(email)`). Collision policy settled:
      the email namespace is shared by ALL providers (setup / oidc / dev) and
      the first claimant wins; a later claim -- a `/auth/setup` insert or an
      OIDC login refreshing its email onto a squatted address -- hits the
      unique violation, which `map_email_conflict` maps to `409 Conflict`
      ("email already registered") instead of an opaque `500`; surfacing the
      squat beats silently rebinding the email. `get_user_by_email` (the
      sharing resolver) now matches case-insensitively, consistent with the
      index, so at most one row can match. Guards:
      `setup_account_duplicate_email_returns_409`,
      `create_user_duplicate_email_is_conflict`,
      `oidc_login_with_squatted_email_is_conflict`,
      `get_user_by_email_is_case_insensitive`.
      (`server/migrations/20261024000000_initial_schema.sql`, index `users_email_lower_unique`,
      `db/users.rs`, `routes/tests.rs`)
