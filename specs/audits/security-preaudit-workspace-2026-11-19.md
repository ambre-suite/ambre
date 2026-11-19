# AMBRE -- SECURITY AUDIT: THE WORKSPACE PRE-AUDIT OUTSIDE THE CALL PATH, FIRST PASS

> An addendum to [`security-audit.md`](security-audit.md) and to
> [`security-audit-blind-2026-11-17.md`](security-audit-blind-2026-11-17.md), whose identifiers it
> continues.

This addendum holds the first pass of the MVP Story A security pre-audit of the workspace outside
the call path. It is an audit alone: this pass fixes nothing, and ticks no box. Its coverage is
partial, as its last section says, and four further passes split by scope follow it.


Nothing was run, built or modified. Every finding below comes from reading the code; where a step rests on something I did not read, the entry says so. Identifiers continue the blind pass (`P66` was its last), with the prefix `W`.

## Summary

| Scope | Files read (whole or the cited ranges) | High | Medium | Low | Info |
| :-- | :-- | :-- | :-- | :-- | :-- |
| 1. Zero-knowledge boundary | `ambre-spec-zero-knowledge.md`, `ambre-spec-message-bus.md` (contract to 1.6), both audits; server `main.rs`, `routes/mod.rs`, `extract.rs`, `ws/handler/mod.rs`, `ws/handler/message.rs` (to l.1000), `ws/pubsub_hub.rs` (fan-out only) | 0 | 0 | 1 (`W70`) | 0 |
| 2. Keys, rotation, logs | `ambre-crypto`: `keys.rs`, `hash.rs`, `passphrase.rs`; `ambre-sync`: `local_store.rs`, `auth.rs` (grep), `membership_log.rs` 596–705; server `routes/keys.rs`, `routes/share.rs`, `jwt.rs` (to l.230), `auth.rs`; `dev-login` feature wiring | 0 | 1 (`W67`) | 1 (`W69`) | 0 |
| 3. Server routes and auth | `routes/auth.rs`, `documents.rs`, `sync.rs`, `media.rs`, `orgs.rs`, `read_cursors.rs`, `channels.rs` (265–335, 735–995), `middleware/rate_limit.rs`, `redis_ops.rs` (1–700, 1140–1282), `db/content.rs` (215–269, 560–699), `db/channels.rs` (`register_channel`), `deletion.rs`, schema (users, channel documents) | 0 | 1 (`W68`) | 1 (`W71`) | 2 |
| 4. Parsers | `ambre-docx/src/xml.rs` 20–309; bound constants of `opc`, `odt`, `pptx`, `xlsx`, `convert` by grep | 0 | 0 | 1 (`W72`) | 0 |
| 5. Main thread, workers, bus | `ambre-app/src/signin.rs`, `scripts/trunk_post_build.py` (policy), `ambre-prose/.../clipboard/html_write.rs`; storage, attachment and showcase paths by grep | 0 | 0 | 0 | 0 |
| **Total** | | **0** | **2** | **4** | **2** |

No finding closed by the earlier audits regressed on the paths I read. `N34` (the development sign-in) holds: `dev_seed` sits behind `cfg(any(test, feature = "dev-login"))` and no default feature turns it on. `N7`/`N38` hold in `create_document`. I did not re-test `P33`–`P66`, which the blind pass lists as open.

## Findings

### W67 — medium — A share by address wraps the key for a self-asserted account, and the client drops the proof flag

- **Where:** `ambre-server/src/routes/documents.rs:68` (`setup_account`), `routes/share.rs:159`–`172` and `:244`–`248`, `db/users.rs:104`, `:209`. No client crate reads `email_verified` (grep over `crates/`: server and the primitives allowlist only).
- **Rule:** `SRV-029`, and its rationale in `ambre-spec-session.md` section 4: "letting anyone assert any address turns a signup form into a way to squat a person's identity and receive documents meant for them".
- **Path:**
  1. Mallory posts `/api/auth/setup` with Bob's address and keys of her own. The route needs no session and no proof of the address.
  2. Alice shares a document with Bob's address. `get_user_by_email` answers Mallory's account.
  3. The relay answers `email_verified: false`. The Sync worker never decodes that field, so the share dialog shows the same fingerprint step as for a verified account.
  4. Alice's client wraps the document key for Mallory's exchange key. Mallory opens the document.
  5. Bob later signs in through his provider and reclaims the address (`SRV-029`). The reclaim renames Mallory's address and nothing else, so she keeps every member row and every wrapped key.
- **Impact:** an outsider reads documents meant for a person who has not signed up yet. The relay is honest throughout. The only defence is an out-of-band fingerprint check on a first contact, with nothing telling Alice that this contact is the risky kind.
- **Fix:** carry the proof flag through the worker to the share dialog, and refuse or require an explicit confirmation before wrapping for a self-asserted address. On a reclaim, tell the inviters of the squatting account, or remove its member rows that were granted by address.

### W68 — medium — After a reset of the ephemeral store, a fully trimmed document restarts its numbering and loses new deltas

- **Where:** `ambre-server/src/redis_ops.rs:213`–`221` (`seed_clock_state` does nothing with no author clock), `:61`–`67`; `db/content.rs:598`–`609` (`get_clock_state` reads `updates` alone), `:654`–`662` (purge at or below the confirmed horizon); `ws/handler/message.rs:215`–`233`; `routes/sync.rs:311`–`317`.
- **Rule:** `SRV-032` (an edit after a reset is accepted, not refused for ever), `SRV-127` (the trim drops only what a confirmed snapshot folds in).
- **Path:**
  1. A document has a confirmed snapshot at horizon N, and the compaction pass has trimmed every `updates` row at or below N.
  2. The ephemeral store is reset. The code plans for this event elsewhere (`SRV-123`, `SRV-114`).
  3. A member subscribes. The reseed reads `updates`, finds no row, and returns without setting the counter or any clock.
  4. A member who never wrote sends clock 1. The script accepts it and `INCR` answers sequence 1.
  5. The delta is stored inline in its row. The next compaction pass deletes it, because 1 ≤ N.
  6. A member who wrote before holds clock k+1 and is refused with "expected 1". I did not read what the client does with that refusal.
- **Impact:** sealed edits are lost for every client that was not live. No attacker triggers the reset, so this is a robustness defect rather than an attack. A second effect: while the counter reads 0, `create_snapshot` bounds no `covered_seq`, so an editor can register a horizon of any size. The code comment itself says that stops the relay asking for snapshots.
- **Fix:** seed the counter from the highest of the `updates` rows and the snapshot horizons, and keep per-author high-water marks somewhere the trim does not delete. Refuse a horizon while the counter is cold instead of skipping the bound.

### W69 — low — Two accounts may register one signing key, and no route asks for proof of the key

- **Where:** schema `20261024000000_initial_schema.sql:16`–`28` (no unique index on `signing_pubkey`); `routes/documents.rs:68`, `routes/keys.rs:26` (neither asks for a signature); `routes/share.rs:46`–`59`; `ws/handler/message.rs:406`, `:661`; `redis_ops.rs:1147`, `:1222`.
- **Rule:** `SYNC-215` and `SYNC-217` (a key is bound to one member), and the relay-side check of `SRV-104` that an entry names "the key that member registered". It turns `P58` from an editor's act into an outsider's.
- **Path:**
  1. Mallory reads Alice's signing key from any member list, or from `GET /users/{id}/pubkeys`.
  2. She sets up an account with that signing key and an exchange key of her own.
  3. An honest owner invites her. `checked_entry` compares the entry's `member_key` with the registered key, finds them equal, and stores a valid entry binding Alice's key to Mallory.
  4. Per `P58`, the latest binding of a key wins in the client directory, so Alice's deltas resolve to no author or to the wrong one. I did not re-read that client code; this step rests on the blind pass.
- **Verified on the relay:** the presence field, the session count and `PeerLeft` are keyed by the signing key, so the two accounts share them. Mallory's socket passes the author-key check for any genuine sample of Alice that she replays.
- **Impact:** wrong or missing attribution of Alice's edits in that document. Mallory cannot sign as Alice, so she forges nothing.
- **Fix:** a unique index on `signing_pubkey`, and a signature over the account identifier or a relay challenge at setup and at the first key store.

### W70 — low — The default log filter is `debug`, the level where the relay logs the membership graph

- **Where:** `ambre-server/src/main.rs:48` (`ambre_server=debug` when no filter is set); for example `routes/share.rs:225` (document, invitee, inviter), `routes/auth.rs:107` and `:255` (user), `routes/channels.rs:820` and `:924`, `ws/handler/mod.rs:402` (document and author key on every delta).
- **Rule:** the letter of `ZK-030` holds, since it binds "info or above". Its stated purpose does not: "the observability stack keeps no part of the membership graph". `ambre-spec-server.md:483` puts identifiers at debug on purpose.
- **Path:** an operator starts the relay with no `RUST_LOG`. Every invitation, sign-in, join and delta writes a line naming who did what on which document to a log store that outlives the relay's own data.
- **Impact:** the graph `ZK-030` keeps out of logs is in them by default. The gate reads the macro level, not the effective filter.
- **Fix:** default to `info`, and require an explicit opt-in for debug outside `DEV_MODE`.

### W71 — low — Three gaps in the rate limiter

- **Where:** `ambre-server/src/middleware/rate_limit.rs:163`–`173`, `:36`–`38`, `:208`, `:235`–`243`.
- **Rule:** `SRV-094`, `SRV-019`, `SRV-013`.
- **Paths:**
  1. Behind a declared proxy, the bucket is the leftmost `X-Forwarded-For` entry. Most proxies append rather than overwrite, so that entry is the caller's to write. A fresh value per request gives a fresh bucket, which defeats the auth tier and creates one sorted set per value.
  2. The auth tier is 10 per minute per address and fails closed. `GET /api/auth/config` needs no session and counts against it. One client behind a shared address (carrier NAT, an office) spends the budget and blocks sign-in and refresh for everyone behind it; their sessions end when their tokens expire.
  3. The script adds the member before it counts, so a refused request is still recorded. A flooded bucket grows with the flood for the length of the window.
- **Impact:** brute-force limits bypassed in a common deployment shape; a cheap targeted lockout; store memory growth.
- **Fix:** take the entry a configured number of hops from the right. Count the config route in the API tier. Count before adding, and add only when the request is allowed.

### W72 — low — The XML tree builders bound depth and part size, not node count

- **Where:** `ambre-docx/src/xml.rs:224`–`305`; `ambre-opc/src/lib.rs:45` (32 MiB per part). `ambre-odt/src/xml.rs` and `ambre-pptx/src/read/xml.rs` declare the same `MAX_DEPTH` and no node bound by grep; I did not read their bodies.
- **Rule:** the hostile-package bounds of `ambre-opc`. I did not read `ambre-spec-interop.md`, so I cite no row.
- **Path:** a `.docx` whose `document.xml` is 32 MiB of `<a/>` deflates to a few tens of kilobytes. The builder allocates one `Element` per tag, about eight million of them at some 70 to 80 bytes each, before any reader sees the tree. Entities are sound: a document type is refused.
- **Impact:** the converter instance asks for several hundred megabytes and traps on a phone, or keeps that memory for the session. The loss of the converter thread is `P66`'s consequence.
- **Fix:** count nodes and attribute bytes while building, and refuse past a bound sized to the model.

### Info

- `redis_ops.rs:658`–`665`: a refresh record is written, then given its lifetime in a second command. A relay that dies between the two leaves a refresh token that never expires.
- `routes/media.rs:136`–`175`: a writer confirms a media address it never uploaded. The upload route then refuses that address to every member (`SRV-271`), so the file can no longer be attached to that document.

## Checked and found sound

- `POST /channels` with another channel's identifier: the unique indexes on one `meta` and one active segment make the transaction roll back.
- The provider return (`signin.rs`): constant-time state, `https` only, address cleaned.
- The page policy the build writes: no inline or evaluated script, no remote image, own-origin connections.
- The Prose clipboard HTML writer: attributes escaped, links and families filtered.
- Murmure attachments: SVG and HTML are never shown inline.
- The access token: algorithm pinned, expiry required.
- The hub fan-out: a full queue drops for that socket alone.
- The local store seal: each row bound to its place.
- Tokens: none reaches browser storage.

## Not read

- `ambre-spec-architecture.md` (asked for in the brief, not opened), `ambre-spec-message-bus.md` past section 1.6, every other spec except the cited lines.
- `ambre-crypto`: `envelope.rs`, `ecdh.rs`, `aead.rs`, `commitment.rs`, `signature.rs`, `strength.rs`, `keys/recovery.rs`, `peer.rs`.
- `ambre-sync`: everything except `local_store.rs`, the listed range of `membership_log.rs` and greps of `auth.rs`. In particular `sharing.rs`, `key_vouch.rs`, `updates.rs`, `pipeline.rs`, `websocket.rs`, `docfile.rs`, `snapshot_trust.rs`, `http.rs`, `renewal.rs`.
- `ambre-protocol`, `ambre-core`, `ambre-bus`, `ambre-logic`, `ambre-crdt`, `ambre-client-core`, `ambre-chrome`, `ambre-commands`, `ambre-canvas`, `ambre-draw`, `ambre-metrics`, `ambre-primitives`, `ambre-belt`: not read.
- `ambre-server`: `s3.rs`, `oidc.rs`, `secrets/`, `payload.rs`, `hot_tail.rs`, `compaction.rs`, `supervise.rs`, `state.rs`, `error.rs`, `routes/answers.rs`, `ws/handler/signature.rs` and `token_bucket.rs`, most of `db/` (`members.rs`, `documents.rs`, `orgs.rs`, `limit.rs`, most of `channels.rs`), `redis_ops.rs` 700–1140 and past 1282, the segment, fold and thread handlers of `routes/channels.rs`, all tests.
- Parsers: `ambre-convert`, `ambre-odt`, `ambre-pptx`, `ambre-xlsx`, `ambre-pdf`, `ambre-prose-form`, `ambre-opc` bodies, and `ambre-docx` beyond `xml.rs`. The CSV and HTML paste paths were checked for a size bound by grep only.
- Applications: `ambre-trame`, `ambre-toile`, `ambre-prose`, `ambre-murmure` and `ambre-app` beyond the files named in the table. The Murmure per-operation author check, the Loro import and what stays on disk after a sign-out remain uncovered, as in the blind pass.
- Out of scope by the brief: `ambre-call`, `ambre-sfu`, the call media of `ambre-murmure` and `ambre-canvas`, `routes/calls.rs`, `ws/handler/call.rs`.