# AMBRE — TECHNICAL ROADMAP (EPICS)

> **Role of this file.** This is the **single entry point** for cross-cutting scheduling. It carries
> the project's **Epics** (coarse, deliverable increments) and their dependency order. The fine-grained
> backlog (**Stories**, with `[x]`/`[ ]` progress) lives in each spec, referenced here by ID.
>
> **Epic / Story model (Agile).**
> - **Epic** = this file. One Epic = one increment of value. Coarse grain, no per-feature detail.
> - **Story** = a spec's roadmap section. Fine grain, individual checkboxes.
> - **Golden rule (derived status).** An Epic is `[x]` **only when every Story it lists is `[x]`**.
>   The Epic status is *derived*, never hand-set. This is the invariant that keeps this file and the
>   specs from contradicting each other. If a Story is open, its Epic is open.
>
> **How to resume with no prior context.** Read this file top-to-bottom, find the first open Epic
> (`[ ]`), open each Story it lists (spec + section), and work the open Stories. You do not need any
> other memory.
>
> **Strategy:** vertical delivery of Trame (validate the full stack on one product), then horizontal
> expansion. No week estimates: the relative order of Epics and their dependencies is the only binding
> constraint.

---

## Vocabulary mapping (Epic <-> spec phases)

The specs predate this file and use their own phase vocabulary. Mapping:

| This file | Foundation specs (`1-Foundations/`) | Trame specs (`2-Trame/`) |
| :--- | :--- | :--- |
| `E0`/`E1` (done) | `ARCH P0`-`P3`, `CANVAS P0`, `CHROME P1`-`P3`, `UI P0`-`P2`, `SYNC P0`-`P3`, `SRV P0`/`P2` | engine Phase 0/1 (logic), virtual scroll + inline editor (ui Phase 1) |
| `E2` | `CANVAS P0` remaining items | -- |
| `E3` | -- | `trame-logic §14`, `trame-features §10`, `trame-ui §18`, `trame-interop §10` (Phase 1) |
| `E4` | `ARCH P2`/`P5`, `CANVAS P1`, `SYNC P4`, `SRV P1` | -- |
| `E5` | `ARCH P4`, `CANVAS P2` | Trame Phase 2+ |

---

## E0 — Technical Foundation  `[x]` (done)

*The 3-thread stack runs, content renders, chrome is assembled, crypto is proven.*

Stories (all `[x]`):
- `ARCH P0` — shared-memory threading, base crates, typed IPC.
- `CANVAS P0` — GPU rendering pipeline (`FrameHost`/`AppView`, fonts, Trame grid 60+ FPS).
  *(core rendering items; remaining `CANVAS P0` items are tracked in `E2`.)*
- `CHROME P1`->`P3`, `UI P0`->`P2` — Leptos chrome (app shell, commands, input/IME, tokens, a11y).
- `SYNC P0` — standalone E2EE primitives (`ambre-crypto`, compress/seal pipeline, `MasterSeed`).

## E1 — Trame vertical: encrypted, synchronized, live-sync engine  `[x]` (done)

*A Trame skeleton end-to-end encrypted and synchronized via the Blind Relay. Validates the whole stack.*

> **Scope note -- "collaborative" is not yet reachable from the UI.** The real-time sync engine and
> the sharing/revocation flow exist at the bus/sync/server layer (`SYNC P3`), but the **chrome has no
> way to share a document with another user**. So today only multiple sessions of the *same* user
> converge -- not genuine multi-user collaboration. The user-facing sharing UI is an explicit open
> Story under `E2`.

Stories (all `[x]`):
- `ARCH P1`/`ARCH P2` — typed Trame editing (`TrameOutcome`, lossless Logic dispatcher), real
  multiplexed WebSocket, multi-document `Unlock`/`OpenDocument`, client document lifecycle (URL
  routing, client-generated `DocumentId`, lazy server creation, per-document IndexedDB restore).
- `ARCH P3` / `SRV P0` — Blind Relay (PostgreSQL/Redis/S3, multiplexed WS, ACL, Lua clock+seq,
  presigned URLs).
- `SYNC P1`->`SYNC P3` — network sync, cookie-less auth (Bearer JWT + refresh), persistent SPA
  engine, live sync (PubSub hub, WS resilience, hydration), sharing & revocation **flow**
  (DocumentKey rotation, media, anti-replay). *(Engine/protocol only -- no chrome UI yet, see `E2`.)*
- `SRV P2` — server hardening (rate limiting, organizations + Shamir, compaction; CSRF dropped
  with the move to Bearer JWT).

> Dependencies: `SRV P0` precedes the network wiring of `SYNC P2` (need a target relay);
> `SYNC P0` precedes `SYNC P1`-`P3` (crypto before integration).

## E2 — Foundation finishing (minimal)  `[x]` (done)

*Close the visible/blocking foundation gaps. Deliberately minimal: offline durability, audits and
benchmarks are deferred to `E5`.*

Stories:
- **Document sharing UI (chrome)** — invite/revoke collaborators by user, wiring the existing
  `InviteToDocument`/`RevokeFromDocument` flow (`SYNC P3`, sharing flow in `ambre-spec-sync.md §3`)
  to a chrome command + dialog. **Until this exists, genuine multi-user collaboration is unreachable
  from the UI** (see `E1` scope note). Unblocks real two-user testing of Trame.
- `CANVAS P0` — generated canvas tokens (`tokens.ron` -> `theme.rs`) + headless tests (mock
  `QuadRenderer`).
- `ambreFatal` reconciliation — the `panic::set_hook` JS hook is done (`ARCH P1`); the remaining
  part is the canvas terminal screen (`CANVAS P1` item), wired in `host`/`ambre-app`.

## E3 — Trame MVP (usual spreadsheet)  `[ ]` — *core objective*

*Turn the Trame skeleton into a usable spreadsheet. Scope = the consolidated "Phase 1 / MVP" of the
four Trame specs, deduplicated against what `E0`/`E1` already delivered (virtual scroll + inline cell
editor are done).*

Stories:
- `trame-logic §14 Phase 1 & 2` — multi-sheet `LoroDoc` + `SheetIndex`; insert/delete rows/columns; cell merges;
  Undo/Redo (Loro `UndoManager`); complete the function set from 22 to ~50.
  *(parser / bytecode VM / `DependencyGraph` / cycles / `#REF!` are already done -- see that section.)*
- `trame-features §10 Phase 1` — shared filter, simple sort, Undo/Redo commands.
- `trame-ui §18 Phase 1` — sheet tabs, keyboard navigation, formula bar, single-cell selection,
  full `on_logic_message` `TrameEvent` dispatch. *(virtual scroll + inline editor already done.)*
- `trame-interop §10 Phase 1` — CSV import/export. *(basic .xlsx import deferred to `E5`.)*

## E4 — Application expansion  `[ ]` (deferred)

*Deploy the other building blocks on the validated foundation.*

Stories:
- `ARCH P2` — `ambre-prose` (LoroText): second pilot, validates the text/CRDT axis.
- `ARCH P5` — `ambre-toile` (R-Tree + read-only embeds), `ambre-murmure` (channels).
- `SYNC P4` + `SRV P1` — channels & meetings (segments managed by Sync, unread, WebRTC signaling).
- `CANVAS P1` — paths/transform + PDF export (krilla).

## E5 — Polish & hardening  `[ ]` (deferred)

*Launch quality, public Beta, advanced Rendez-Vous.*

Stories:
- `ARCH P4` — offline durability (Loro IndexedDB persistence, flush + `RequestMissing` on reconnect);
  key-rotation hot-path benchmarks + ZK audit (`cargo tree`); wasm-only warnings audit.
- `CANVAS P2` — frame budget profiling, dynamic theme, i18n/RTL, non-regression benchmarks.
- Trame Phase 2+ — basic .xlsx import, conditional formatting, data validation.
- Rendez-Vous — SFrame encryption (E2EE video): `SYNC P4` / Rendez-Vous.
