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
| `E2` | `CANVAS P0` remaining items, `CHROME P4` (launcher) | -- |
| `E3` | -- | `trame-logic §14`, `trame-features §10`, `trame-ui §18`, `trame-interop §10` (Phase 1) |
| `ET` | `ARCH P6` (test infrastructure) | Trame "Tested DoD" debt items (logic/features/ui/interop) |
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

> **Scope note.** The real-time sync engine and the sharing/revocation flow exist at the
> bus/sync/server layer (`SYNC P3`); the user-facing sharing UI that drives them from the chrome
> landed under `E2` (Document sharing UI story). Genuine multi-user collaboration is now reachable
> from the UI.

Stories (all `[x]`):
- `ARCH P1`/`ARCH P2` — typed Trame editing (`TrameOutcome`, lossless Logic dispatcher), real
  multiplexed WebSocket, multi-document `Unlock`/`OpenDocument`, client document lifecycle (URL
  routing, client-generated `DocumentId`, lazy server creation, per-document IndexedDB restore).
- `ARCH P3` / `SRV P0` — Blind Relay (PostgreSQL/Redis/S3, multiplexed WS, ACL, Lua clock+seq,
  presigned URLs).
- `SYNC P1`->`SYNC P3` — network sync, cookie-less auth (Bearer JWT + refresh), persistent SPA
  engine, live sync (PubSub hub, WS resilience, hydration), sharing & revocation **flow**
  (DocumentKey rotation, media, anti-replay). *(Engine/protocol; chrome UI landed under `E2`.)*
- `SRV P2` — server hardening (rate limiting, organizations + Shamir, compaction; CSRF dropped
  with the move to Bearer JWT).

> Dependencies: `SRV P0` precedes the network wiring of `SYNC P2` (need a target relay);
> `SYNC P0` precedes `SYNC P1`-`P3` (crypto before integration).

## E2 — Foundation finishing (minimal)  `[x]` (done)

*Close the visible/blocking foundation gaps. Deliberately minimal: offline durability, audits and
benchmarks are deferred to `E5`.*

> Was reopened for `CHROME P4` (the `/` launcher discarded the E2E-encrypted document name and was
> off the design tokens). Both that launcher gap and the Document sharing UI Story have since landed,
> so the derived Epic status is `[x]` again.

Stories:
- `CHROME P4` `[x]` — **Launcher screen** (`ambre-spec-chrome.md §2.G`/`§7`): Sync now decrypts
  `name_enc` through to `DocumentSummary.name` (ECDH-unwrap each list-row doc key, AEAD-open the name;
  server list returns `doc_key_enc` + `wrapper_exchange_pub_key` + `member_count`), document cards
  (type-icon monogram + name/"Untitled" + relative last-modified + shared badge), create-untitled /
  inline rename / delete (with confirmation `<dialog>`), token re-theme + empty state + sign-in
  wordmark/tagline, and the generic-label browser tab title. Naming/encryption contract in
  `ambre-spec-architecture.md §3`.
- **Document sharing UI (chrome)** `[x]` — Share toolbar button (shown when a document is open) opens
  a dialog that lists current members and wires the existing `InviteToDocument` /
  `RevokeFromDocument` / `UpdateDocumentMemberRole` / `ListDocumentMembers` flow (`SYNC P3`): invite
  by email (bare username -> `<name>@dev.local` in dev mode), per-member role select, revoke (with
  server-side `DocumentKey` rotation), live `InviteSent` / `RevocationComplete` / `MemberRoleUpdated`
  toasts that refresh the list, plus a `DocumentShared` toast that refreshes the launcher. Invite
  does a real X25519 ECDH wrap of the document key for the invitee. Unblocks genuine multi-user
  testing of Trame.
- **Snapshot lifecycle (client legs)** `[x]` — wired the client end of the snapshot
  upload/download flow (`ambre-spec-sync.md §"Snapshot Flow"`, `ambre-spec-server.md §2.B`):
  - **Upload:** `Sync::handle_send_snapshot` now seals -> `POST /snapshot/upload-url` ->
    S3 PUT -> `POST /snapshot` (with `covered_seq`) -> adopts the returned `snapshot_id`
    (`SyncEvent::SnapshotUploaded`). Host path still stops after sealing.
  - **Download + cold-start seq gating:** the open path uses the real routes
    (`GET /snapshot` then `POST /snapshot/download-url`); the snapshot's new `covered_seq`
    (DB column) seeds `last_seq` so the post-open `RequestMissing` replays only newer deltas
    instead of the whole history since genesis. Server replay also parallelized
    (`join_all` over S3 fetches). Closes the cold-start target (`ambre-spec-server.md §perf`).
    *(Remaining post-MVP: BLAKE2b `parent_snapshot_proof` chain, still empty.)*
- `CANVAS P0` `[x]` — generated canvas tokens (`tokens.ron` -> `theme.rs` via `ambre-canvas/build.rs`)
  + headless tests (`RecordingSurface`, the mock `DrawSurface` in `ambre-draw/src/testing.rs`,
  consumed by the Trame render tests).
- `ambreFatal` reconciliation `[x]` — the `panic::set_hook` JS hook (`ARCH P1`) plus the canvas
  terminal screen: `signal_fatal` is raised in `ambre-app/src/lib.rs` and `fatal_message` is painted
  every frame in `ambre-app/src/host.rs`.

## E3 — Trame MVP (usual spreadsheet)  `[ ]` (reopened) — *core objective*

*Turn the Trame skeleton into a usable spreadsheet. Scope = the consolidated "Phase 1 / MVP" of the
four Trame specs, deduplicated against what `E0`/`E1` already delivered (virtual scroll + inline cell
editor are done).*

> **Reopened** for the header/column/row UX gap. The functional MVP shipped, but the structural-op
> UX is below the bar set by the autofill handle: insert/delete live only in an unstyled context
> menu, there is no whole-column/row selection, no hover affordance, no resize, and the filter gives
> only a faint funnel badge. The promotion also pulls **multi-column sort** and **column/row resize**
> forward from V1 into the MVP. Per the golden rule the Epic status is derived, so adding open
> Stories flips E3 back to `[ ]`.

Stories:
- **Header/column/row UX (MVP promotion)** `[ ]` — `trame-ui §18 Phase 1` (new items) + `trame-logic §14 Phase 1` + `trame-features §10 Phase 1`:
  - Header interactions (`trame-ui §4bis`): virtual whole-column/row selection, hover cursors, **column/row resize** (`ResizeCol`/`ResizeRow` + variable-metrics `GridLayout`, `trame-logic §9/§14`), **double-click auto-fit** (UI-side over cached cells, reuses `ResizeCol`/`ResizeRow`; whole-column auto-fit is a V1 refinement), inline "+" insert affordance.
  - Context-menu polish (`trame-ui §5.C`): grouped iconed entries + separators + header-scoped menu.
  - **Multi-column sort** (`SortRangeMulti`, `trame-features §2`): `SortDialog` UI + logic handler. *(was V1.)*
  - Filter feedback on the first row (`trame-ui §5.D`): Excel-style autofilter buttons fed by `FilterApplied { col }`.
- *(Below: the original E3 scope, delivered.)*
- `trame-logic §14 Phase 1 & 2` — multi-sheet `LoroDoc` + `SheetIndex`; insert/delete rows/columns; cell merges;
  Undo/Redo (Loro `UndoManager`); complete the function set from 22 to ~50; manual cell formatting
  (`SetCellStyle` + `NumberFormat`), clipboard paste (`PasteRange`), autofill (`FillRange`),
  find/replace (`ReplaceAll`/`FindNext`), selection aggregates (`RequestSelectionStats`).
  *(parser / bytecode VM / `DependencyGraph` / cycles / `#REF!` are already done -- see that section.)*
- `trame-features §10 Phase 1` — shared filter, simple sort, Undo/Redo commands.
- `trame-ui §18 Phase 1` — sheet tabs, keyboard navigation, formula bar, single + multi-cell selection,
  formatting toolbar (bold/colors/H+V alignment/number format incl. currency/percent/scientific/fraction/date/time/custom/borders/wrap), copy/cut/paste, autofill,
  find/replace, cell wrap + vertical alignment + overflow, selection status bar,
  full `on_logic_message` `TrameEvent` dispatch. *(virtual scroll + inline editor already done.)*
- `trame-interop §10 Phase 1` — CSV import/export.

## ET — Testing & CI foundation  `[x]` (transverse) — *settles the MVP test debt*

*Testability is a non-negotiable pillar (`ambre-context.md §3`, template §4, `ambre-spec-architecture.md §4`). The Trame MVP (`E3`) shipped functional but under-tested: ~20 server DB/redis tests are bare-`#[ignore]`, there is no CI, no measured coverage, no CRDT convergence property-test, and zero Leptos UI test. This Epic makes the existing tests run and closes the gaps before stacking new apps on `E4`. Per the tested Definition of Done (template §4), the apps it touches are only truly "done" once this is green.*

Stories:
- `ARCH P6` `[x]` — CI GitHub Actions (`lint` / `test-host` / `test-db` / `test-wasm` / `coverage`) with postgres/redis/minio services; root `justfile` for local/CI parity.
- `ARCH P6` `[x]` — feature `db` on `ambre-server`; replace the ~20 bare `#[ignore = "needs postgres/redis"]` with the `cfg_attr(not(feature = "db"), ignore = ...)` gate so the already-written server tests run; `cargo-llvm-cov` host baseline + lcov artifact.
- `trame-logic §14` `[x]` — 2-peer CRDT convergence property-test (`proptest`), targeting references-after-sort and structural-op reconciliation. Landed as `ambre-trame/src/logic/convergence_tests.rs::two_peers_converge_under_interleaved_ops`.
- `trame-ui §18` `[x]` — Leptos UI tests (`wasm-bindgen-test`, headless Chrome): reactive headless + real DOM for the Trame chrome. Landed as `ambre-trame/tests/chrome_ui.rs` (reactive belt + DOM `StatusBar`/`FindReplace`), run via `just test-wasm` in the re-enabled CI job.
- `trame-features §10` / `trame-interop §10` `[x]` — wired the "Tested DoD" belts green in CI: the MVP filter/sort/undo `#[test]`s (added the three Phase-1 undo belts), the concurrent-filter LWW case folded into the 2-peer convergence property-test (`ApplyFilter`/`ClearFilter` ops + `is_filtered` in the converged snapshot), and the CSV roundtrip + malformed/byte-soup fuzz robustness cases. *(Conditional-formatting / validation / protection tests follow their Phase 2 / Post-MVP implementation.)*

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
