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

## E4 -- TOILE MVP  `[x]`

An infinite collaborative whiteboard with a presentation mode. What is new is the spatial document
model and the GPU vector canvas.

> **Dependency.** `ET` precedes it: every story ships with its test belt green.
>
> **Auto-layout and boolean ops are out of the MVP, frozen.** Toile is a Miro, not a Figma
> (`features/ambre-spec-toile-engine.md section 5`), so a constraint solver and vector boolean ops
> are another product's metier. They stay in the tree with no stabilization budget, as candidates for
> V2 or for removal.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` The board engine | MVP | elements, connectors, freehand strokes, groups, frames, z-order, undo, clipboard, find, selection stats |
| `[x]` `CANVAS P1` -- the vector pipeline | MVP | shapes and paths on the GPU, pan and zoom, colour text, PDF output |
| `[x]` The board surface | MVP | the tool set, the inline text editor, the tool palette, the style inspector, the floating selection toolbar, the menu bar, the find panel, the status bar, live peer cursors |
| `[x]` Presentation and reactions | MVP | the presenter panel, playback, Smart Animate between slides, follow the presenter, emoji reactions and their badges |
| `[x]` Toile interop | MVP | the SVG and image converters, the SVG serializer, the media upload round trip and the PDF pages of a deck, all Logic-side |
| `[x]` The new-document type picker | MVP | create any kind of document from the launcher, not only a spreadsheet |
| `[x]` `ARCH P6` -- the cross-app plane | MVP | who a peer is, what they are allowed to do, and the plane that runs one document inside another: its own session, its own permission, its own event stream, painted and driven by the launcher |
| `[x]` Comments | MVP | anchored comment threads with replies and resolve, their pins on the board, and the side panel |
| `[x]` Live presence on the board | MVP | this peer's cursor, selection and stroke reach the others, the highlighter becomes a tool, a peer who leaves stops being painted, and a label is a name. The render half ships and receives nothing |
| `[x]` Order the deck | MVP | a panel that adds a frame to the presentation, reorders it and removes it, and a deleted frame that leaves it. Playback, the transition and the speaker notes ship and have no deck to run on |
| `[x]` The Toile interop surface | MVP | a file dropped or picked reaches the importer at the drop point, the SVG and PDF export entries hand the bytes to the browser, and the lossy-import report reaches the user |
| `[x]` Embeds on the board | MVP | a gesture places a web page or another Ambre document on the board, and a web page renders sealed off from it. The nesting plane ships -- opened, focused, painted and routed -- and nothing puts an embed there |
| `[x]` Connector labels | MVP | a caption on an arrow, editable in place |
| `[x]` An arrow can be selected | MVP | click an arrow to select it, then delete, re-aim, restyle or reorder it |
| `[x]` Hyperlinks and go-to-frame | MVP | a link on an element, followed to a URL or to another frame |
| `[x]` Align and distribute | MVP | align or space a selection evenly, as one undo item |
| `[x]` No delta is lost on an export error | MVP | Toile drops a change when an export fails; Trame retries it. Align Toile on Trame |

## ET -- TESTING AND CI FOUNDATION  `[x]`

The MVP shipped functional and under-tested. This epic settled that debt before stacking new apps.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` `ARCH P5` -- CI and the test tiers | -- | the pipeline, the host, database and wasm tiers, one recipe per job for local parity, a coverage baseline |
| `[x]` `ARCH P5` -- the database tier runs | -- | the twenty server tests that were skipped by default now run against real services |
| `[x]` The convergence property test | -- | two peers, interleaved operations, identical converged documents |
| `[x]` The first UI belt | -- | the Trame chrome tested in a real browser, reactive layer and DOM |
| `[x]` The MVP feature belts | -- | the filter, sort, undo and CSV belts green in CI, and the concurrent-filter case folded into the convergence test |

## EF -- DOCUMENT CORPORA AND NATIVE FILE FORMAT  `[x]`

Documents in known states, and the file they travel in. One epic because a fixture **is** an `.ambre`
file and the fixture loader **is** the production importer.

> **Gates.** `DOCFILE P1` follows `ARCH P8`. A file on a user's disk has unbounded age and is
> beyond our reach to migrate. It must not ship before payload migration has a mechanism. `EF` does
> not block `E4`.
>
> **`FIXTURES P2` is the one story exempt from "green in CI".** Its benches read git-ignored corpora
> no CI job builds, so the condition is unreachable by construction. It ships `[x]` when
> `just fixtures-bench` runs clean on a developer machine and its baselines are committed with the
> machine they came from. A silent exemption is how a rule stops meaning anything, so the substitute
> is named here.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` `DOCFILE P0` -- the container | -- | the `.ambre` file: one header, one version, one migration path, plaintext export and import |
| `[x]` `FIXTURES P0` -- deterministic generation | -- | a document built from a seed is byte-identical every time, on any machine |
| `[x]` `FIXTURES P1` -- the corpus and its gate | -- | the committed test documents, the interop export sources, and a CI gate that fails when one drifts |
| `[x]` `CONTRIBUTING.md` | -- | the file, which does not exist, and the two checklists that need somewhere to live |
| `[x]` `ARCH P8` -- persisted schema versioning | -- | a document says which schema it holds; an older one migrates, a newer one is refused without damage, and a record the client cannot decode reaches the user instead of vanishing (`SCHEMA-003` to `SCHEMA-010`) |
| `[x]` `DOCFILE P1` -- user-facing export and import | V1 | save a document to a file and open one back, in any app |
| `[x]` `DOCFILE P2` -- passphrase-sealed files | V1 | an exported file encrypted under a passphrase |
| `[x]` `FIXTURES P2` -- scale corpora and benches | V1 | documents at the sizes the performance budgets name, and the benchmarks that read them |
