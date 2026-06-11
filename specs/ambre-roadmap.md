# AMBRE — TECHNICAL ROADMAP

> **Role of this file.** It carries the **cross-cutting order** of the project: the sequence of
> inter-crate *milestones* and their dependencies. It does not re-enumerate any task.
> The **detail and progress status** (`[x]`/`[ ]`) lives in §7 of each spec in
> `1-Foundations/`, referenced here by prefixed ID (`ARCH P1`, `SYNC P2`, `SRV P0`,
> `CANVAS P0`, `CHROME P1`, `UI P1`).
>
> **Strategy:** vertical delivery of Trame (validate the full stack on one product),
> then horizontal expansion. No week estimates: the relative order of milestones
> and their dependencies is the only binding constraint.

---

## M0 — Technical Foundation

*Objective: the 3-thread stack runs, content renders, chrome is assembled, crypto is proven.*

- [x] Shared-memory threading + base crates + typed IPC: `ARCH P0`, `ARCH P1` (partial).
- [x] GPU rendering pipeline (`FrameHost`/`DocView`, fonts, Trame grid 60+ FPS): `CANVAS P0`.
- [x] Leptos Chrome (app shell, commands, input/IME, tokens, a11y): `CHROME P1`→`P3`, `UI P0`→`P2`.
- [x] Standalone E2EE primitives (`ambre-crypto`, compress/seal pipeline, `MasterSeed` derivation): `SYNC P0`.

## M1 — Vertical Trame encrypted & synchronized

*Objective: a 100% functional Trame, end-to-end encrypted and synchronized via the Blind Relay.*
*This is the milestone that validates the entire stack.*

- [x] **Typed Trame editing** (`TrameOutcome`, lossless Logic dispatcher): `ARCH P2`.
- [x] **Real network sync**: non-blocking loop + multiplexed WebSocket, `Unlock`/`SetupAccount`,
  `OpenDocument`/`CloseDocument`, server-side document CRUD: `ARCH P1` (remaining items), `SYNC P1` (wired) → `SYNC P2`.
- [x] **Blind Relay**: PostgreSQL/Redis/S3, multiplexed WS, ACL, Lua clock+seq, presigned URLs: `SRV P0`.
- [x] **Canvas finishing**: generated tokens, headless tests: `CANVAS P0` (remaining items).
- [x] **Client document lifecycle**: URL routing (`/` landing list, `/trame/<docid>`),
  client-generated `DocumentId` (UUID v7), lazy server creation on first edit
  (`POST /api/documents` idempotent with client id), per-document IndexedDB restore,
  `OpenDocument` tolerant of a not-yet-created (404) document. Replaces the hardcoded
  smoke document as the real entry path (smoke doc retained as headless-test fixture).

> Dependencies: `SRV P0` precedes the network wiring of `SYNC P2` (need a target relay);
> `SYNC P0` precedes `SYNC P1`/`P2` (crypto before integration).

## M2 — Identity, sharing & hardening

*Objective: secured multi-user, sharing/revocation, foundation ready for Enterprise.*

- [x] Sharing & revocation (Document Key rotation, media, anti-replay): `SYNC P3`.
- [x] Server hardening (rate limiting, CSRF, organizations + Shamir, compaction): `SRV P2`.
- [x] Offline durability (Loro IndexedDB persistence, flush + `RequestMissing` on reconnect): `ARCH P4`.
- [x] Full Trame formula engine (DAG, incremental recomputation, cycle detection): `ARCH P4`.

## M3 — Application expansion

*Objective: deploy the other building blocks on the validated foundation.*

- [ ] `ambre-prose` (LoroText), `ambre-toile` (R-Tree), `ambre-murmure` (channels): `ARCH P2`/`ARCH P5`.
- [ ] Sync channels/meetings (segments managed by Sync, WebRTC signaling): `SYNC P4`.
- [ ] Server channels + meetings (segments, unread, signaling): `SRV P1`.
- [ ] Canvas paths/transform + PDF export Krilla: `CANVAS P1`.

## M4 — Polish & Advanced Rendez-Vous

*Objective: launch quality, public Beta.*

- [ ] Frame budget profiling, dynamic theme, i18n/RTL, non-regression benchmarks: `CANVAS P2`.
- [ ] Rendez-Vous: SFrame encryption (E2EE video): `SYNC P4` / Rendez-Vous.
- [ ] Final ZK security audit, wasm-only warnings audit: `SYNC P4`, `ARCH P4`.
