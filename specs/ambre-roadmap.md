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

| This file | Foundation specs (`1-Foundations/`) | Trame specs (`2-Trame/`) | Toile specs (`3-Toile/`) |
| :--- | :--- | :--- | :--- |
| `E0`/`E1` (done) | `ARCH P0`-`P3`, `CANVAS P0`, `CHROME P1`-`P3`, `UI P0`-`P2`, `SYNC P0`-`P3`, `SRV P0`/`P2` | engine Phase 0/1 (logic), virtual scroll + inline editor (ui Phase 1) | -- |
| `E2` | `CANVAS P0` remaining items, `CHROME P4` (launcher) | -- | -- |
| `E3` | -- | `trame-logic §14`, `trame-features §10`, `trame-ui §18`, `trame-interop §10` (Phase 1) | -- |
| `ET` | `ARCH P6` (test infrastructure) | Trame "Tested DoD" debt items (logic/features/ui/interop) | -- |
| `E4` | `ARCH P5` (`ambre-toile`), `CANVAS P1` (paths/PDF) | -- | `toile-logic §13`, `toile-features §9`, `toile-ui §13`, `toile-interop §10` (Phase 0/1) |
| `E5` | `ARCH P2`/`P5` (`ambre-prose`, `ambre-murmure`), `SYNC P4`, `SRV P1` | -- | -- |
| `E6` | `ARCH P4`, `CANVAS P2` | Trame Phase 2+, `trame-logic §14 Phase 4` (scaling debt) | Toile Phase 2 (optimization) |

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
benchmarks are deferred to `E6`.*

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

## E3 — Trame MVP (usual spreadsheet)  `[x]` (done) — *core objective*

*Turn the Trame skeleton into a usable spreadsheet. Scope = the consolidated "Phase 1 / MVP" of the
four Trame specs, deduplicated against what `E0`/`E1` already delivered (virtual scroll + inline cell
editor are done).*

> **Reopened** (first pass) for the header/column/row UX gap — multi-column sort, column/row resize,
> whole-axis selection, autofilter buttons: all landed, that Story is `[x]`.
>
> **Reopened again** (second pass) for a chrome UX polish round, tracked by the "Chrome UX polish
> pass" Story below. Every item in it has now landed, so per the golden rule the derived Epic status
> is `[x]` again.

Stories:
- **View-independent merge masking (correctness)** `[x]` — `trame-logic §14 Phase 1`
  (finding `trame-logic §11 bis.E`). A masked cell (covered by a merge, non-empty -> conflict) was
  excluded from the formula engine **only on the active sheet** (`reseed_engine` skipped
  `masked_cells` at local index 0), so a cross-sheet reference (`=Sheet2!A1`) to a conflicted cell
  evaluated to a **different value depending on which sheet was displayed** -- a formula result that
  depended on view state, contradicting `trame-logic §6` ("masked cells do not participate in any
  computation"). Wrong at any document size, so it was an MVP correctness item, not scaling debt.
  Fixed: masking is derived per sheet from `merges_{sheet}` at feed time (`masked_cells_for`) and
  excluded uniformly on every sheet; the cross-sheet regression
  `masked_cell_excluded_across_sheets_regardless_of_active_tab` proves a conflicted masked cell reads
  identically regardless of the active tab.
- **Chrome UX polish pass** `[x]` — bring the grid/chrome interaction quality up to the bar set by
  the rest of the MVP (`trame-ui §4bis`/`§5.C`/`§5.D`/`§13`):
  - `[x]` Autofilter glyphs (button + column-letter badge + in-button inset) scale with the grid zoom.
  - `[x]` Unified neutral hover on a single generated `--surface-hover` token (`tokens.ron`),
    shared by every chrome control (toolbar, status bar, menus, launcher cards, sheet tabs).
  - `[x]` **Sheet-tab strip migrated from canvas to DOM chrome**: an app-agnostic `<Tabs>` AppShell
    slot + a `SheetTabs` component (tabs, "+", inline rename, `{Rename,Delete}` menu, HTML5
    drag-reorder via the existing `MoveSheet`), fed by `SheetList`-mirrored signals. The canvas tab
    rendering / hit-testing / input routing is gone; `grid_bottom` now spans the full canvas.
  - `[x]` Header separators: hairlines at each visible column's right edge and each visible row's
    bottom edge, so the letters/numbers read as discrete buttons (`render/headers.rs`, drawn
    independently of `text_enabled` so the PDF path keeps them).
  - `[x]` Context-aware right-click menu: `CtxScope::{Cell, Column, Row, Corner}` (set by the view's
    right-click `PointerDown`, read through a launcher-owned `RwSignal`) selects the entry layout in
    `chrome::commands::context_entries_for`, with axis-plural structural labels and the dynamic
    `filter_entry` (**Filter by column** XOR **Remove filter**, never both). A row menu only ever
    offers to drop an active filter, since filtering is column-oriented.
  - `[x]` In-grid header hover feedback: `HeaderHit`-derived `hover_col` / `hover_row` /
    `hover_corner` tint the hovered column, row or select-all corner on the canvas with the
    `header_hover` palette entry, generated from `tokens.ron` on the same value as the chrome's
    `surface_hover`, and always drawn under the active tint so the active axis wins.
  - `[x]` Inter-column/row "+" insert affordance: a badge pinned to the hovered header boundary
    (`insert_hint` -> `draw_insert_badge`), clicking it inserts at that edge and selects the new
    column/row. *(The whole-axis drag-reorder of grid columns/rows shipped separately — see the E6
    story, pulled into the MVP.)*
- **Header/column/row UX (MVP promotion)** `[x]` — `trame-ui §18 Phase 1` (new items) + `trame-logic §14 Phase 1` + `trame-features §10 Phase 1`:
  - Header interactions (`trame-ui §4bis`): virtual whole-column/row selection, hover cursors, **column/row resize** (`ResizeCol`/`ResizeRow` + variable-metrics `GridLayout`, `trame-logic §9/§14`), **double-click auto-fit** (UI-side over cached cells, reuses `ResizeCol`/`ResizeRow`; whole-column auto-fit is a V1 refinement), inline "+" insert affordance.
  - Context-menu polish (`trame-ui §5.C`): grouped iconed entries + separators + header-scoped menu.
  - **Multi-column sort** (`SortRangeMulti`, `trame-features §2`): `SortDialog` UI + logic handler. *(was V1.)*
  - Filter feedback on the first row (`trame-ui §5.D`): Excel-style autofilter buttons fed by `FilterApplied { col }`.
> **Known schema deferral (does not gate `E3`).** `trame-logic §14 Phase 0` stays `[~]`: the
> `{v,t,s}` cell sub-map and the shared `styles` map of `§2.B` were never implemented. The MVP is
> functionally complete without them, and the 2026-07-09 review split the remainder in two — the
> style fan-out consolidation (an `E6` Story, gated on the public Beta) and the `styles` dictionary
> (deferred behind a benchmark). See `trame-logic §11 bis.F`.

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

*Testability is a non-negotiable pillar (`ambre-context.md §3`, template §4, `ambre-spec-architecture.md §4`). The Trame MVP (`E3`) shipped functional but under-tested: ~20 server DB/redis tests are bare-`#[ignore]`, there is no CI, no measured coverage, no CRDT convergence property-test, and zero Leptos UI test. This Epic makes the existing tests run and closes the gaps before stacking new apps (`E4` onward). Per the tested Definition of Done (template §4), the apps it touches are only truly "done" once this is green.*

Stories:
- `ARCH P6` `[x]` — CI GitHub Actions (`lint` / `test-host` / `test-db` / `test-wasm` / `coverage`) with postgres/redis/minio services; root `justfile` for local/CI parity.
- `ARCH P6` `[x]` — feature `db` on `ambre-server`; replace the ~20 bare `#[ignore = "needs postgres/redis"]` with the `cfg_attr(not(feature = "db"), ignore = ...)` gate so the already-written server tests run; `cargo-llvm-cov` host baseline + lcov artifact.
- `trame-logic §14` `[x]` — 2-peer CRDT convergence property-test (`proptest`), targeting references-after-sort and structural-op reconciliation. Landed as `ambre-trame/src/logic/convergence_tests.rs::two_peers_converge_under_interleaved_ops`.
- `trame-ui §18` `[x]` — Leptos UI tests (`wasm-bindgen-test`, headless Chrome): reactive headless + real DOM for the Trame chrome. Landed as `ambre-trame/tests/chrome_ui.rs` (reactive belt + DOM `StatusBar`/`FindReplace`), run via `just test-wasm` in the re-enabled CI job.
- `trame-features §10` / `trame-interop §10` `[x]` — wired the "Tested DoD" belts green in CI: the MVP filter/sort/undo `#[test]`s (added the three Phase-1 undo belts), the concurrent-filter LWW case folded into the 2-peer convergence property-test (`ApplyFilter`/`ClearFilter` ops + `is_filtered` in the converged snapshot), and the CSV roundtrip + malformed/byte-soup fuzz robustness cases. *(Conditional-formatting / validation / protection tests follow their Phase 2 / Post-MVP implementation.)*

## E4 — Toile MVP (infinite whiteboard + presentations)  `[ ]` (in progress) — *current core objective*

*Second product on the validated stack: an infinite collaborative whiteboard with a presentation mode.
Mirrors the `E3` model — scope = the consolidated **Phase 0 + Phase 1** of the four Toile specs
(`3-Toile/`). Toile reuses the whole `E0`/`E1` foundation (threading, sync, blind relay, crypto,
launcher/sharing UI); what is new is the spatial document model and the GPU vector-canvas.*

> Dependencies: `ET` (test foundation) precedes it — every Story ships with its test belt green, per the
> tested DoD. Toile's PDF export and vector tools carry `CANVAS P1` (paths/transform + krilla), so that
> foundation Story lands here rather than under `E5`.

Stories:
- `[~]` `ARCH P5` (`ambre-toile`) — new spatial `DocState`: typed `ToileState` behind the `ambre-logic` `match`,
  `rstar` R-tree culling, in-place editable nested embeds (Prose/Trame). *(embeds start at the read-only subset.)*
  Standalone `ToileState` + `SpatialIndex` landed; `ambre-logic` `match` dispatch **landed** (IPC contract moved to
  `ambre-bus::app::toile`, `AppCommand::Toile`/`AppEvent::Toile`, `DocState::Toile`, remote/snapshot/ephemeral legs).
  Open: in-place editable nested embeds (`ToileCommand::Embedded`).
- `[ ]` `CANVAS P1` — `lyon` / `WgpuSurface` vector pipeline (quad + path), `ViewportTransform`, glyph atlas, and
  PDF export via `KrillaSurface` (krilla). The shared canvas foundation the vector tools and the PDF path need.
- `[~]` `toile-logic §13 Phase 0/1` — CRDT schema + `SpatialIndex` + element/connector/group/stroke lifecycle +
  clipboard + `DisplayViewport`, then dispatcher/Sync wiring, `apply_ephemeral` / `apply_remote`, snapshot &
  persistence paths, `my_user_id` from `SessionReady`, `UndoManager`, Find + selection stats, `UserId` presence.
  Includes the 2-peer convergence property-test (**DoD**). *(Phase 0 landed: schema, index, element/connector/
  stroke lifecycle, groups, `UndoManager`, `DisplayViewport`, convergence property-test green. Phase 1 dispatcher/
  Sync/persistence wiring landed (dispatch arm, `SendUpdate`/`SendEphemeral`, `apply_remote`/`apply_remote_batch`,
  snapshot + local-restore paths). Open: clipboard, frames content, Find; `my_user_id` from `SessionReady`,
  selection-stats plumbing to the status bar, and `UserId` presence.)*
- `[ ]` `toile-ui §13 Phase 0/1` — `ViewportTransform` / batcher / adaptive grid / hit-testing / Select tool, then
  the full tool set (Pen/Shape/Connector/Draw/Sticky/Text), Parley inline editor, shortcuts + OS-clipboard
  mirror, context menu / find panel / status bar, `PeerPresence` overlay with cross-app `UserId` colors,
  wasm render/input tests.
- `[ ]` `toile-features §9 Phase 0/1` — presentation sequence CRDT + playback machine, speaker notes, Smart Animate
  + auto-layout solvers, reactions/votes CRDT keyed by `UserId` + derived tally, boolean ops; then playback
  wired to the camera pass, presenter overlay, reactions render, "follow presenter" ephemeral broadcast.
- `[ ]` `toile-interop §10 Phase 0/1` — SVG import/export, raster import, safety limits, media hashing + placeholder;
  then PDF export (presentation order), Figma/Miro converters, media pipeline on the crypto/Sync layer.
  Golden fixtures + wasm export tests.

## E5 — Application expansion  `[ ]` (deferred)

*Deploy the remaining building blocks on the validated foundation.*

Stories:
- `ARCH P2` — `ambre-prose` (LoroText): the text/CRDT pilot.
- `ARCH P5` (`ambre-murmure`) — channels.
- `SYNC P4` + `SRV P1` — channels & meetings (segments managed by Sync, unread, WebRTC signaling).

## E6 — Polish & hardening  `[ ]` (deferred)

*Launch quality, public Beta, advanced Rendez-Vous.*

Stories:
- `ARCH P4` — offline durability (Loro IndexedDB persistence, flush + `RequestMissing` on reconnect);
  key-rotation hot-path benchmarks + ZK audit (`cargo tree`); wasm-only warnings audit.
- `CANVAS P2` — frame budget profiling, dynamic theme, i18n/RTL, non-regression benchmarks.
- **Trame scaling & consistency debt (2026-07 architecture review)** `[ ]` —
  `trame-logic §14 Phase 4` (findings detailed in `trame-logic §11 bis`). The identity layer
  (`Uid`/`SheetIndex`/`AxisId`) is sound; the debt is recomputation strategy, fixable without a
  document-format migration: **incremental remote apply** via Loro subscription diffs (kill the
  O(document) `reseed_engine` on every remote batch / undo / rename -- restores the `§8 P3`
  contract) + ranged display invalidation replacing the per-batch `DataReset`; **cached per-sheet
  `SheetIndices`** (currently rebuilt on every write/scroll -- P1 budget, restores the `§7` state
  shape); **stable engine sheet indices** (de-pin the active sheet from index 0 so `ActivateSheet`
  stops resetting the engine); **dependency-graph micro-fixes** (O(N) `remove`, whole-graph cycle
  check, full-DAG topo walk, dead write-back loop, `formula_cache` `RefCell`).
  *(View-independent merge masking, being a correctness defect rather than scaling debt, moved to
  the `E3` Story below. Bold/italic folding is absorbed by the style-consolidation Story below.)*
- **Trame per-cell style consolidation (schema, `trame-logic §11 bis.F`)** `[ ]` — a cell's style is
  scattered over up to 11 per-sheet `LoroMap`s (`format_`, `style_`, `fgcolor_`, `bgcolor_`,
  `bordcolor{0..3}_`, `font_`, `numfmt_`) instead of the single style reference `trame-logic §2.B`
  prescribes. Costs: `set_cell_style` touches 9 containers per cell and commits **per cell** inside
  the selection loop; `format_{sheet}` survives as a vestigial bold/italic mask duplicating
  `style_{sheet}`; no style deduplication. Target: one style container per cell holding the whole
  `CellStyle`, one transaction per selection.
  **Not a correctness defect** — the suspicion that concurrent whole-cell style edits could converge
  to a per-container blend was tested and refuted (a whole-cell edit writes every container in one
  monotonic-Lamport burst, so Loro's LWW elects the same winner in each); locked in by
  `logic/convergence_tests.rs::concurrent_whole_cell_style_edits_do_not_blend_across_containers`.
  **Ordering:** it does *not* preempt `E5`, but it changes the persisted schema, so it must land
  **before the public Beta**, while no third-party document exists and no migration path has to be
  carried forever. *(The shared `styles` dictionary + `"s"` indirection and the `{v,t,s}` cell
  sub-map are explicitly out of scope: deferred behind a benchmark — Loro does not GC orphaned map
  entries, so the indirection would force a style-reclamation policy.)*
- Trame Phase 2+ — basic .xlsx import, conditional formatting, data validation.
- **Trame column/row reorder (drag & drop)** `[x]` — Excel-style drag to move a whole
  column/row to a new position. **Pulled into the MVP** once investigation showed it was far
  cheaper than assumed: cells are keyed by move-invariant `(row_uid, col_uid)` pairs and the
  formula engine bytecode is uid-anchored, so a reorder is a single `LoroMovableList::mov` that
  rewrites **no** references -- exactly the sort machinery (`realize_sort`), which already proves
  "references follow the data" (§12). No `formula/shift.rs` change was needed (that module is
  paste-only). Shipped as an atomic `MoveColumn` / `MoveRow` op reusing the shared `after_reorder`
  tail (rebuild caches + recompute, crucially *not* `reseed_engine`), plus the 2-peer convergence
  case in `logic/convergence_tests.rs` (concurrent moves converge by LWW on list order) and a
  header drag-and-drop with an insertion guide line. *(This is the grid column/row reorder, distinct
  from the sheet-tab reorder, which is plain `MoveSheet` and already ships in the DOM `SheetTabs`.)*
- Rendez-Vous — SFrame encryption (E2EE video): `SYNC P4` / Rendez-Vous.
