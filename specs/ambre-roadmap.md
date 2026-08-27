# AMBRE -- ROADMAP

**Type:** Scheduling (one file for the whole project)
**Scope:** every epic and every story, for every application.

> **Role of this file.** Nothing schedules anywhere else. A spec says what the system does; this file
> says what work is left to reach it. A spec that carries its own backlog is a second schedule, and
> two schedules disagree.

---

## 1. THE MODEL

| Term | What it is | Where it lives |
| :--- | :--- | :--- |
| **Feature** | the smallest thing a user can name, ship alone and specify end to end | one spec document, always current |
| **Story** | one unit of production that moves one or more features forward | one row of this file |
| **Epic** | a group of stories, one increment of value | one heading of this file |
| **Milestone** | a release scope spanning epics | the `Milestone` column |

**Six rules, and each one exists because it was broken.**

* **A story is one row, and it says what, never how.** A checkbox, a name, its milestone, the
  features it touches. The code says how it works and git says how it got there. A row that explains
  an implementation is a row nobody reads, and it rots on the next commit while looking
  authoritative.
* **A story that needs a paragraph is two stories**, or it is design content sitting in the wrong
  file.
* **Finishing a story is ticking its box. Nothing else.** No note added, no wording changed, no
  account of what was done. The row said what the work was before it started, and it still says it.
* **Closing an epic is cutting it into `ambre-roadmap-journal.md`, verbatim. Nothing else.** Every
  row as written, the heading as written. No summary, no retrospective, no rewrite. An epic is `[x]`
  only when every story under it is `[x]`: the status is derived, never set. While the epic is open a
  finished story stays, ticked, because that is what lets a reader say "eight of eleven, these three
  remain".
* **A milestone tags a story, never a feature.** A feature lives: it has an MVP form, then others.
  `--` means no release scope names the story, which is why transverse work waits.
* **An epic holds no `post-V1` story.** One such row holds an epic open for ever, so section 10
  carries them.

**An epic key is an identifier, not a rank.** `EF` precedes `E4`, and `E4` is the current objective. A
key is minted once, because a few hundred invariant rows cite one. The `Milestone` column orders the
work.

A story is cited by its name, and the name is a join key: `invariants.py` fails on a citation this
file does not answer (`ambre-spec-template.md` section 1, `ambre-spec-architecture.md` section 4).

---

## 2. SHIPPED

Closed epics live in `ambre-roadmap-journal.md`, whole. They are named here so a citation resolves.

| Epic | What it delivered |
| :--- | :--- |
| `E0` | the three-thread stack, the GPU pipeline, the chrome, the crypto primitives |
| `E1` | the Trame vertical end to end: encrypted, synchronized, live through the Blind Relay |
| `E2` | the launcher, the sharing UI, the snapshot client legs |
| `E3` | the Trame MVP: a usable spreadsheet |
| `ET` | CI, the three test tiers, the convergence property test, the first UI belts |

---

## 3. EF -- DOCUMENT CORPORA AND NATIVE FILE FORMAT  `[ ]`

Documents in known states, and the file they travel in. One epic because a fixture **is** an `.ambre`
file and the fixture loader **is** the production importer.

> **Gates.** `DOCFILE P1` follows `ARCH P8`: a file on a user's disk has unbounded age and is beyond
> our reach to migrate, so it must not ship before payload migration has a mechanism. `EF` does not
> block `E4`.
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
| `[ ]` `FIXTURES P1` -- the corpus and its gate | -- | the committed test documents, the interop export sources, and a CI gate that fails when one drifts |
| `[ ]` `CONTRIBUTING.md` | -- | the file, which does not exist, and the two checklists that need somewhere to live |
| `[ ]` `ARCH P8` -- persisted schema versioning | -- | a document says which schema it holds; an older one migrates, a newer one is refused without damage, and a record the client cannot decode reaches the user instead of vanishing (`SCHEMA-003` to `SCHEMA-010`) |
| `[ ]` `DOCFILE P1` -- user-facing export and import | V1 | save a document to a file and open one back, in any app |
| `[ ]` `DOCFILE P2` -- passphrase-sealed files | V1 | an exported file encrypted under a passphrase |
| `[ ]` `FIXTURES P2` -- scale corpora and benches | V1 | documents at the sizes the performance budgets name, and the benchmarks that read them |

---

## 4. E4 -- TOILE MVP  `[ ]`  *current core objective*

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
| `[ ]` Comments | MVP | anchored comment threads with replies and resolve, their pins on the board, and the side panel |
| `[ ]` Live presence on the board | MVP | this peer's cursor, selection and stroke reach the others, the highlighter becomes a tool, a peer who leaves stops being painted, and a label is a name. The render half ships and receives nothing |
| `[ ]` Order the deck | MVP | a panel that adds a frame to the presentation, reorders it and removes it, and a deleted frame that leaves it. Playback, the transition and the speaker notes ship and have no deck to run on |
| `[ ]` The Toile interop surface | MVP | a file dropped or picked reaches the importer at the drop point, an export entry hands the bytes to the browser, and the lossy-import report reaches the user. Every converter ships and no gesture calls one |
| `[ ]` Embeds on the board | MVP | a gesture places a web page or another Ambre document on the board, and a web page renders sealed off from it. The nesting plane ships -- opened, focused, painted and routed -- and nothing puts an embed there |
| `[ ]` Connector labels | MVP | a caption on an arrow, editable in place |
| `[ ]` Hyperlinks and go-to-frame | MVP | a link on an element, followed to a URL or to another frame |
| `[ ]` Align and distribute | MVP | align or space a selection evenly, as one undo item |
| `[ ]` No delta is lost on an export error | MVP | Toile drops a change when an export fails; Trame retries it. Align Toile on Trame |

---

## 5. E5 -- APPLICATION EXPANSION  `[ ]`

Prose, then Murmure. Prose is the text pilot: it validates the axis neither Trame's grid nor Toile's
plane touches, and it unblocks Toile's inline rich text and Trame's text colours.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` `ARCH P2` -- the Prose engine core | MVP | the document model: text, blocks, marks, nesting |
| `[ ]` Pagination | MVP | measured pages: breaks, orphans and widows, kept-together blocks, footnote placement, incremental relayout |
| `[ ]` Styles, lists and headings | MVP | character and paragraph styles, ordered and bulleted lists, numbered headings |
| `[ ]` Tables, table of contents and word count | MVP | tables with merged cells and styles, a generated table of contents, a live word count |
| `[ ]` Headers, footers, find and replace | MVP | page headers and footers with dynamic fields, and find and replace |
| `[ ]` Comments and track changes | MVP | anchored comments, and suggested changes a reviewer accepts or rejects |
| `[ ]` Undo, footnotes and cross references | MVP | undo isolated to the local author, footnotes, cross references |
| `[ ]` Page rendering | MVP | pages on the canvas with virtual scrolling and cached text |
| `[ ]` Input and cursor | MVP | typing, caret navigation, click and drag selection, optimistic display |
| `[ ]` Toolbar, slash menu and composition | MVP | the contextual format bar, the slash menu, input-method composition, double-click word selection |
| `[ ]` Collaborative cursors and annotations | MVP | remote carets, comment highlights, track-change highlights |
| `[ ]` Ruler, clipboard and inertial scroll | MVP | the interactive ruler, copy and paste, selection-aware editing, inertial scroll |
| `[ ]` The native format | MVP | a Prose document saved to and read from an `.ambre` file |
| `[ ]` Markdown interop | MVP | Markdown import and export, with a report of what was lost |
| `[ ]` HTML interop | V1 | HTML import and export, including a paste from a web page |
| `[ ]` DOCX interop | V1 | Word import and export, comments and track changes included |
| `[ ]` PDF export | V1 | a paginated PDF with working links and metadata |
| `[ ]` Images | V1 | images in the document, resolved through the shared media flow and embedded on export |
| `[ ]` `ARCH P7` -- Murmure | V1 | chat channels, and a typed command surface in place of an opaque byte payload |
| `[ ]` `SYNC P4` / `SRV P1` -- channels and meetings | V1 | channel membership and history, unread tracking, and meeting signalling |

---

## 6. E6 -- V1 COMPLETION  `[ ]`

### Foundations

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` `ARCH P4` -- offline durability | -- | edit while offline and lose nothing on reconnect |
| `[ ]` `CANVAS P2` -- frame quality | -- | frame profiling, allocation-free painting, dynamic theme, content accessibility, translation and right-to-left |
| `[ ]` The icon pipeline | -- | vector icons on the canvas. The last open item of `CANVAS P0` |
| `[ ]` Clipped paths on the canvas | -- | a shape crossing a clip edge is drawn whole today. Clip it, and give a stroke a screen-constant width |
| `[ ]` The adaptive chrome | V1 | the navigation, the side panels, the toolbar overflow and the wide data views adapt at the three declared widths. Today two labels and one bottom sheet move, and the regions the design names exist at no width |
| `[ ]` The stylesheet reads the tokens | V1 | fifty literal colours and five undeclared custom properties in the shell stylesheet bypass the one token source, so a theme switch moves everything except them |

### Trame

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` One style register per cell | -- | a cell's whole style converges as one value, so no concurrent edit can tear it |
| `[x]` Column and row reorder | MVP | drag a whole column or row to a new position, references following the data |
| `[ ]` Incremental remote apply | -- | a remote change costs the cells it touched, and the UI re-fetches the range that changed instead of the whole viewport |
| `[ ]` Cached sheet indexes | -- | a scroll on one sheet no longer rebuilds every other sheet's index |
| `[ ]` Stable engine sheet indexes | -- | switching a tab no longer resets the engine |
| `[ ]` A recompute bounded by what changed | -- | the cycle check and the topological walk cover the cells that changed, not the whole graph |
| `[ ]` Engine completion | V1 | whole-column ranges, reference-preserving functions, the advanced function set, convergent deletion of the last sheet |
| `[ ]` Data tools | V1 | personal filter views, conditional formatting, data validation, and a total that skips filtered rows |
| `[ ]` Grid surface | V1 | frozen rows and columns, function autocompletion, live peer cursors, an auto-fit that measures the whole column instead of the viewport cache |
| `[ ]` xlsx interop | V1 | Excel import and export: values, formulas, styles, merges, column widths |
| `[ ]` Cell comments | V1 | a note anchored to a cell, surviving a sort and a reorder |
| `[ ]` Media in a sheet | V1 | Trame declares images and resolves none. Wire it to the shared media flow |
| `[ ]` Persisted page setup | V1 | paper size, margins and orientation stored with the document instead of asked at export |
| `[ ]` A machine-readable error | V1 | an error carries a code a host can branch on, not only a sentence |
| `[ ]` Bounded float objects | V1 | an absurd image or chart size cannot make layout cost unbounded |

### Toile

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Snap to the board grid | V1 | a drag, a resize and a nudge quantize to the grid step the board asks for, which is drawn today and magnetic nowhere |
| `[ ]` Rich text on the board | V1 | a note body converges character by character and carries per-run formatting, on the shared text substrate Prose unblocks. Today it is one whole value and one style per element |
| `[ ]` Engine and render optimization | -- | a large board stays fluid: bulk index updates, cheaper layer diffs, memoized routing, atlas eviction. It is where the parent-lookup scan, the paint-order rewrite and the uncalled connectors land |
| `[ ]` Large-file import | -- | a very large import stays responsive and reports progress |
| `[ ]` Figma and Miro import | V1 | open a board exported from either, reporting every construct it had to drop |
| `[ ]` PNG export | V1 | a board or a frame rendered back to a raster image, without blocking the visible frame |
| `[ ]` The standalone showcase bundle | V1 | a board exported as a self-contained page that renders without the relay. It leaves the zero-knowledge boundary, and says so |

---

## 7. E7 -- CORRECTNESS AND SECURITY DEBT  `[ ]`

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` A lost edit is never shown as applied | V1 | a full command channel drops a lossless command in both applications: Toile logs a warning inside its edit send, and Trame returns the error to a call site that logs it and then writes the same text into its own view state, so the user reads an edit the document never received. The undo group boundary is discarded outright beside it (`BUS-011`) |
| `[ ]` One clipboard host | V1 | the asynchronous write replaces the ownership token the copy event wrote, so a same-tab paste reaches the worst rung of the fidelity ladder; and the toolbar Paste button is a second path that skips the ladder and names one application inside the launcher |
| `[ ]` `SYNC P4` hardening | -- | backpressure on the socket, which the two-instance split made unreachable rather than unbuilt; the access token that rides the socket address into the console; the snapshot adopted as a parent before it is downloaded; and the two tokens a sign-out drops without wiping |
| `[ ]` The production sign-in path | V1 | the two provider routes are registered under a fixed provider name while their handlers read one from the path, so every call answers an internal error and the only sign-in a deployment offers has never run; and the rate limiter sits inside the nested router, which strips the prefix its classifier matches on, so no request is ever counted against the brute-force tier |
| `[ ]` A document is compacted as its history grows | V1 | the relay never asks anyone for a snapshot, and that request is the only trigger of the client's whole compaction path. So no document is ever compacted: every cold open replays from the first delta, the delta index is never trimmed, and the cold-start budget is unreachable |
| `[ ]` A deleted document leaves nothing | V1 | deleting a document removes its rows and neither its sealed objects nor its ordering state, so every snapshot, delta and picture stays in storage for ever and the sequence counters stay in the ephemeral store with no expiry |
| `[ ]` A revoke either completes or changes nothing | V1 | the removal is posted first and the rotation can fail after it, so the member is out of the list and their key still opens new content. A member whose exchange key does not parse is skipped and loses the document in silence |
| `[ ]` The rotation re-keys the whole document | V1 | the document name and every media blob stay sealed under the previous key, so the first revoke blanks the name and every picture for everyone who stayed; and no snapshot seals the state after the rotation |
| `[ ]` The media address is not the plaintext | V1 | a blob is addressed by the hash of its plaintext and the relay stores it, so the relay can confirm that a document holds a file it already knows. The media type travels beside it, and a blob is bound to neither its document nor its address |

---

## 8. E8 -- THE SHARED MODELS ACROSS THE APPS  `[ ]`

Each row promotes a behaviour shared by nature into one model, so the editors stop reinventing it
apart. Arbitrated in the 2026-08 cross-app review; the target state is in the Foundations spec that
owns the subsystem.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` One command registry | MVP | one description per command -- title, icon, category, chord -- with the menus, the keyboard and the command palette all derived from it instead of built beside it |
| `[ ]` One find and replace | V1 | one match session with a live cursor, in every app, instead of Trame's whole-grid re-scan |
| `[ ]` One zoom model | V1 | one zoom engine behind the shared widget: Trame gains keyboard and pinch zoom and zoom-to-selection, Prose gains zoom. Touch and pinch belong here, in every app, and to no other row |
| `[ ]` One text-editing substrate | V1 | one editing core under both layout drivers: the input pipe, composition, the caret and selection model |
| `[ ]` One interop vocabulary | V1 | the same import and export events, the same conversion report, the same progress model, in every app |
| `[ ]` One document locale | V1 | the document carries its locale, so collaborators on differently configured machines read identical numbers and dates |
| `[ ]` One resize command | V1 | one row and column resize primitive for both table models. No shared table object |
| `[ ]` One media cache | V1 | every decoded image goes through one bounded cache, Toile and Trame included |
| `[ ]` One repaint contract | V1 | Prose and Toile honour the shared partial-repaint modes instead of a parallel mechanism |
| `[ ]` One high-density rule | V1 | the shared surface scales for a high-density display once, so no app spec restates it |
| `[ ]` One selectable PDF | V1 | one glyph-to-character mapping in the shared surface, so exported text is selectable and searchable in every app instead of drawn shapes alone |
| `[ ]` One undo bound | V1 | a bounded undo depth in every app. What merges into one step stays per app |
| `[ ]` One keymap | V1 | the `Ctrl` and `Cmd` fold, both redo chords, and the legacy copy and paste chords, decided once |
| `[ ]` One name for the moving selection end | V1 | `focus` everywhere; the fixed end stays `anchor`. A rename |
| `[ ]` One floating-menu shell | V1 | one placement, flip and keyboard model under every popover and floating bar |
| `[ ]` One performance read-out | V1 | the frame read-out available in every app, not in Trame alone |
| `[ ]` Links in Trame | V1 | clickable per-cell links, on the link and open-URL model shared with Prose and Toile |

---

## 9. E9 -- THE VERIFICATION DEBT  `[ ]`

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Hot-path benchmarks and the dependency audit | -- | measured crypto costs, and a machine check that no plaintext crate reaches the boundary |
| `[ ]` The name resolver reads the manifests | -- | five `Cargo.toml` files justify a dependency by naming the spec that needs it, and `spec_refs.py` opens neither a manifest nor any file outside `crates/**/*.rs` and `specs/**/*.md`, so those citations are held by nothing |
| `[ ]` The performance budgets given a mechanism | -- | about thirty budgeted operations across the seven tables of `ambre-spec-budgets.md`, held by one bench. A budget that cannot fail a build is not verified |
| `[ ]` The review status means something | -- | every spec reads `Status: Draft`. The two other values `ambre-spec-template.md` declares are defined by no row and set by no one, so the corpus cannot tell a reviewed document from an unread one |
| `[ ]` Visual tests of canvas content | -- | no canvas content is verifiable in CI: headless browsers read back a blank frame |
| `[ ]` The Sync and relay belts | -- | the shipped rows no test would fail on, across the Sync worker, the sealed envelope, the delta order, the membership, the key rotation, the media transfer, the relay, its session, its sequencer and its stores |
| `[ ]` The UI-thread belts | -- | the same debt on the frame, the chrome shell, the clipboard, the input pipe, the launcher, the key verification, the design tokens, the render assets and accessibility |
| `[ ]` The Trame belts | -- | the same debt across the grid viewport, the cell surface, the number formats and the data tools |
| `[ ]` The Toile belts | -- | the same debt across the board surface, the connectors, the presentation and the interop converters |
| `[ ]` Fuzzing the untrusted readers | -- | a `cargo-fuzz` target on the wire decode, the `.ambre` container and the formula parser, each of which is fed by a relay or by a user's file |
| `[ ]` The suite checked adversarially | -- | `cargo-mutants` over the crates whose tests are generated, so a test that asserts nothing fails |
| `[ ]` The workspace lints | -- | `forbid(unsafe_code)` in every crate instead of two, and a workspace lint table in place of the default clippy set |
| `[ ]` Snapshot tests of the wire and the surfaces | -- | `insta` over the recording surface's operation sequences, the wire encodings and the PDF primitives |
| `[ ]` A coverage gate on added lines | -- | `cargo-llvm-cov` in diff mode, which turns a baseline nobody reads into a check |
| `[ ]` The browser-driven belt | -- | a served build driven by browser automation: the `ambre-app` boot, one command across the canvas and DOM seam, and the dialogs and menus the unit runner cannot drive |
| `[ ]` A real object-store round trip | -- | a presigned upload against a local minio in the database tier, which performs none today |

---

## 10. POST-V1 -- NOT SCHEDULED

### Foundations

| Story | Touches |
| :--- | :--- |
| `[ ]` Split-secret organization recovery | the escrow has a release path and no deposit path, and the quorum is clamped to one so the register advertises no guarantee the relay cannot keep. Threshold reconstruction, and a deposit, are what make the clamp liftable |
| `[ ]` Rendez-Vous | end-to-end encrypted video |
| `[ ]` Cross-app rich paste | a paste from one app into another keeps its structure instead of falling back to plain text |

### Prose

| Story | Touches |
| :--- | :--- |
| `[ ]` The later writing set | regular-expression find, bookmarks, version history, linked styles, pageless mode, line numbers, tables of figures, an index, a bibliography, section breaks and columns, protected zones, an equation editor |
| `[ ]` The spell checker | misspelled words underlined, with suggestions |

### Trame

| Story | Touches |
| :--- | :--- |
| `[ ]` Range protection | a range only its owner may edit, and the refusal the others see |
| `[ ]` Data tools, second pass | colour scales, data bars, top-N formats, and validation by range, date or pattern |
| `[ ]` Sheet PDF export | print a sheet to a vector PDF |
| `[ ]` Post-V1 surface | sparklines, charts, reference autocompletion, smooth scroll |
| `[ ]` Post-V1 interop | OpenDocument import, and the Google Sheets bridge |
| `[ ]` Version history | named snapshots a user can return to, in place of an undo stack lost on close |
| `[ ]` Engine optimization | the indexes and parallelism a million-cell sheet needs, measured against benchmarks |
| `[ ]` Recompute on mobile hardware | measure the recompute on a target device. Past 50 ms at P95 it must yield instead of blocking |
| `[ ]` The shared style dictionary | the style indirection the schema prescribes and no code writes. Gated on a benchmark: it needs a reclamation policy first |

### Toile

| Story | Touches |
| :--- | :--- |
| `[ ]` The facilitation layer | template boards, cards with tags, a shared timer, a voting session and the surface that casts a vote, a minimap |

---

## 11. HOW TO RESUME WITH NO PRIOR CONTEXT

Read section 1, then the epic marked *current core objective*, and work its open rows. Open a spec
only when you start the row that names it. No other file schedules anything.
