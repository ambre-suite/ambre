# AMBRE -- ROADMAP JOURNAL (SHIPPED EPICS)

**Type:** Scheduling (closed epics)
**Scope:** every epic that has closed, one row per story.

> **Role of this file.** An epic is cut here from `ambre-roadmap.md` the day it closes, verbatim.
> Every row as written, the heading as written -- no summary, no retrospective, no rewrite. It
> answers one question: what shipped, and under which epic. Nothing schedules from here.
>
> **A row says what, never how.** The code says how it works, and git says how it got there. A row
> that explains an implementation is a row nobody reads, and it goes stale on the next commit while
> looking authoritative.

---

## E0 -- TECHNICAL FOUNDATION  `[x]`

The three-thread stack runs, content renders, the chrome is assembled, the crypto is proven.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` `ARCH P0` -- threading and the base crates | -- | shared-memory threading, the crate split, typed inter-thread messages |
| `[x]` `CANVAS P0` -- the render pipeline | -- | the GPU pipeline, the frame host, fonts, the Trame grid above 60 frames per second |
| `[x]` `CHROME P1` to `P3`, `UI P0` to `P2` -- the chrome | -- | the app shell, the command registry, input and composition, design tokens, accessibility |
| `[x]` `SYNC P0` -- the crypto primitives | -- | the seal and open envelope, key derivation from a master seed, standalone and network-free |

## E1 -- TRAME VERTICAL, ENCRYPTED AND SYNCHRONIZED  `[x]`

One product end to end through the Blind Relay. This is the epic that proved the whole stack.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` `ARCH P1` / `ARCH P2` -- typed editing and the document lifecycle | -- | the Trame command surface, the multiplexed WebSocket, multi-document open and close, URL routing, local restore |
| `[x]` `ARCH P3` / `SRV P0` -- the Blind Relay | -- | the relay itself: documents, access control, the update log, snapshots in object storage |
| `[x]` `SYNC P1` to `SYNC P3` -- network sync | -- | delta sync, cookie-less authentication, live sync with reconnection, sharing and revocation with key rotation, media |
| `[x]` `SRV P2` -- server hardening | -- | rate limiting, organizations with split-secret recovery, compaction, load tests |

## E2 -- FOUNDATION FINISHING  `[x]`

Deliberately minimal: close the visible gaps, defer durability and benchmarks.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` `CHROME P4` -- the launcher | -- | a real document home: decrypted names, document cards, create, rename, delete |
| `[x]` The sharing UI | -- | invite by email, per-member roles, revoke with key rotation, live notifications |
| `[x]` The snapshot client legs | -- | snapshot upload and download, and the cold-start replay gate that stops a full history replay |
| `[x]` Canvas tokens and headless render tests | -- | generated canvas tokens, and a recording draw surface that tests the real renderer with no GPU |
| `[x]` The fatal-error screen | -- | a panic reaches the user as a terminal screen instead of a dead canvas |

## E3 -- TRAME MVP  `[x]`

A usable spreadsheet.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` The workbook model | MVP | multiple sheets, the sheet lifecycle, insert and delete of rows and columns, cell merges, undo and redo |
| `[x]` The formula engine | MVP | the parser, the bytecode machine, the dependency graph, cycle detection, broken references, about 113 functions, named ranges, cross-sheet references |
| `[x]` Cell editing | MVP | the inline editor and formula bar, clipboard copy, cut and paste, autofill, find and replace, selection aggregates |
| `[x]` Formatting | MVP | manual cell formatting and the toolbar, number formats, wrap, alignment, empty-neighbour overflow |
| `[x]` Data tools | MVP | the shared filter, single and multi-column sort, autofilter buttons on the header row |
| `[x]` The grid surface | MVP | virtual scrolling, keyboard navigation, single and multi-cell selection, sheet tabs, the context menu, the status bar |
| `[x]` Header, column and row interactions | MVP | whole-axis selection, hover cursors, resize, double-click auto-fit, the inline insert affordance |
| `[x]` CSV interop | MVP | CSV import and export |
| `[x]` View-independent merge masking | MVP | a masked cell is excluded from every computation on every sheet, so a formula result no longer depends on the active tab |
| `[x]` Chrome UX polish | MVP | zoom-aware glyphs, one shared hover token, the sheet-tab strip in the DOM, header separators, a context menu aware of what was clicked |

## ET -- TESTING AND CI FOUNDATION  `[x]`

The MVP shipped functional and under-tested. This epic settled that debt before stacking new apps.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` `ARCH P5` -- CI and the test tiers | -- | the pipeline, the host, database and wasm tiers, one recipe per job for local parity, a coverage baseline |
| `[x]` `ARCH P5` -- the database tier runs | -- | the twenty server tests that were skipped by default now run against real services |
| `[x]` The convergence property test | -- | two peers, interleaved operations, identical converged documents |
| `[x]` The first UI belt | -- | the Trame chrome tested in a real browser, reactive layer and DOM |
| `[x]` The MVP feature belts | -- | the filter, sort, undo and CSV belts green in CI, and the concurrent-filter case folded into the convergence test |
