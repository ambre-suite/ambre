# AMBRE -- THE BUDGET REGISTER

**Type:** Register (every budget of the workspace)

> Every performance and resource target Ambre commits to, one row each. A rule that must hold a
> budget cites the row here instead of restating the figure.

* **Every budget of the workspace is a row here.** A budget a reader cannot compare to its
  neighbour is a number, not a target. No component spec carries one of its own, and a rule that
  must hold a budget cites the row instead (`CANVAS-005`, `FIXTURE-007`). `Method` names the mechanism
  that measures the row, and "**none exists**" is the honest value.
* **This file is a register, not a specification.** It states no rule of its own, so it declares no
  contract table: `ambre-spec-template.md section 4` holds what a file of this class owes.

---

## 1. THE CLIENT, PER OPERATION (TARGET: MOBILE HARDWARE 2024)

| Operation | Target (P95 mobile) | Ceiling | Thread | Method |
|-----------|-------------------|---------|--------|---------|
| queue send/receive UI <-> Logic (`ambre_bus::conduit`, shared heap, by move) | < 1 us | -- | UI <-> Logic | benchmark |
| Logic <-> Sync `postMessage` (postcard encode + structured clone / transfer) | < 1 ms | -- | Main <-> Sync | benchmark |
| Scroll round-trip UI -> Logic -> UI (P1) | < 2 ms | -- | UI <-> Logic | benchmark e2e |
| Outbound encryption 1 KB (compress+AEAD+HMAC+Ed25519) | < 200 us | < 500 us | Sync | criterion |
| Inbound decryption 1 KB (verify+HMAC+AEAD+decompress) | < 350 us | < 1 ms | Sync | criterion |
| Full crypto round-trip (excluding network) | < 600 us | < 1.5 ms | Sync | criterion |
| Snapshot compression 100 KB (ruzstd) | < 5 ms | -- | Sync | criterion |
| Passphrase-sealed file, one seal or one open (Argon2id, then one AEAD pass over the whole container) | < 1 s | < 2 s | Sync | the login work factor, deliberately: 64 MiB, 3 passes, 1 lane, paid per file rather than per session, because it is also what an offline guessing attack pays (`ambre-spec-docfile.md`, `DOCFILE-014`). The 64 MiB is the largest transient the Sync worker allocates, and it is released with the call -- **none exists** |
| Quarantine release, 100 updates | < 1 ms | -- | Sync | criterion -- **none exists** |
| `doc.import()` Loro update (1 KB delta) | < 2 ms | -- | Logic | criterion |
| Murmure authorship check, one received delta read as a Loro JSON update | < 50 us | -- | Logic | criterion `ambre-murmure` `segment_json_check` -- 2.3 us on a desktop in native code, whatever the size of the segment. The fork it replaced measured 3.5 ms at 500 messages and 51.6 ms at 5000 (`segment_fork_check`). The phone figure assumes a phone about 7 times slower than the desktop: the ratio of the inbound open budget (350 us) to its desktop measure (50 us). The `Murmure measured on a phone` Story (`ambre-roadmap.md` E5) replaces that ratio with a measure |
| Murmure space leave, the rotation of the space key and of every public channel key | < 2 s, for 50 public channels and 200 members, to measure on a phone | -- | Sync | **none exists** -- one wrap per remaining member for each key, so about 10 000 key exchanges at that size. A measure past the target brings back one key for every public channel (`features/ambre-spec-murmure-spaces.md`) |
| Murmure segment, Loro import of its snapshot, 5000 messages | < 10 ms | -- | Logic | criterion `ambre-murmure` `segment_snapshot` -- 3.0 ms on a desktop in native code |
| Murmure segment, Loro replay of one delta past its snapshot | < 250 us | -- | Logic | criterion `ambre-murmure` `segment_replay_one_by_one` -- about 35 us on a desktop in native code: 220 ms for the 6250 deltas of a segment at its close |
| Trame formula DAG recompute (10k cells) | < 50 ms | -- | Logic | custom benchmark -- **none exists** |
| Scroll viewport response (P1, end-to-end) | < 15 ms | -- | Logic -> UI | benchmark e2e |
| Trame CSV import (10k lines) | < 200 ms | -- | Logic | criterion -- **none exists** |
| Trame CSV export (10k lines) | < 100 ms | -- | Logic | criterion -- **none exists** |
| Trame xlsx import (1k cells / 10k cells) | < 500 ms / < 3 s | -- | Logic | criterion -- **none exists**, and no code either |
| Trame xlsx export (10k cells) | < 1 s | -- | Logic | criterion -- **none exists**, and no code either |
| Trame `SetCellText` to viewport (P2) | < 5 ms | -- | Logic | recompiles the bytecode and updates the DAG -- **none exists** |
| Trame batch of 50 cells (P4) | < 1 ms | -- | Logic | **none exists** |
| Trame `SheetIndex` rebuild, 10k rows | < 10 ms | -- | Logic | at load time -- **none exists** |
| Trame `range_index` scan, 1k range formulas | < 1 ms | -- | Logic | linear scan at the MVP, an interval tree later -- **none exists** |
| Toile visible-set query, 50 000 elements | < 1 ms | -- | Logic | must fit inside a frame with margin for tessellation -- **none exists** |
| Toile index update, one element | < 50 us | -- | Logic | a drag mutates one element per input event -- **none exists**, and the code walks the whole index (`TOILE-091`) |
| Toile remote change, delta < 32 KB | < 2 ms | -- | Logic | keeps the client share of the < 300 ms inter-user budget -- **none exists**, and the code rebuilds every cache (`TOILE-092`) |
| Toile transform command (move, restyle) | < 1 ms | -- | Logic | one register write, one index update, one derivation -- **none exists** |
| Toile body commit, a 4 KB body merged from its base | < 1 ms | -- | Logic | two character diffs capped at 256 edit steps, then one diff into the body, per close (`TOILE-UI-381`). The UI echo runs the same two diffs inside its frame (`TOILE-UI-382`) -- **none exists** |
| Toile slide transition, 500 elements per frame | < 4 ms | -- | Logic | leaves room to paint inside the frame during a transition -- **none exists** |
| Toile layout solve, 200 children | < 1 ms | -- | Logic | runs at derivation on every child edit -- **none exists**, and nothing calls the solver (`TOILE-FEAT-029`) |
| Toile reaction tally | < 0.1 ms typical | -- | Logic | one scan of the reactions map on read, never stored -- **none exists** |
| Toile boolean combination, two typical paths | < 5 ms | -- | Logic | user-initiated and one-shot, off the render frame -- **none exists** |
| Toile SVG import, 5 000 nodes | < 500 ms | -- | Logic | one-shot user action, off the render thread, applied in slices between the thread's messages (`TOILE-IO-028`) -- `crates/ambre-toile/benches/import.rs` measures the whole import and one slice, desktop only: about 280 ms whole and 1.7 ms a slice. The slice is what a camera move waits behind; an edit waits for the whole |
| Toile PDF export, 50 slides | < 3 s | -- | Logic | acceptable for an explicit export, off the render frame. It is not off the Logic command loop, so it is also how long the document stops accepting edits (`3-Toile/features/ambre-spec-toile-pdf-export.md`, `TOILE-IO-099`) -- **none exists** |
| Coverage levels across a 30-degree vector edge | >= 8 of 256 | -- | UI | the antialiasing quality figure, measured rather than eyeballed: rendered headless, the alpha histogram along the edge is the mechanism, and a pipeline with no coverage term yields exactly 2 (`CANVAS-061`, `ambre-spec-drawing-surface.md`) -- **none exists** |
| Antialiasing band width, at any zoom | 1.0 physical px +/- 0.25 | -- | UI | measured at both ends of the zoom range on one shape; a band tessellated in model units and not divided by the transform scale fails it as the board zooms in (`CANVAS-063`) -- **none exists** |
| Toile first paint of a picture | < 15 ms | -- | UI | the display-latency budget: the placeholder paints and the network never blocks it -- **none exists** |
| Toile media cache hit on a revisit | 0 refetch | -- | UI | the bounded local cache (`features/ambre-spec-toile-media.md section 3`) -- **none exists** |
| Prose local text insert, transaction plus exported delta | < 2 ms | -- | Logic | **none exists**, and no code either |
| Prose pagination, full recompute of 100 pages | < 20 ms | -- | Logic | placement over already shaped runs, which is why the shaping rows below are charged apart (`features/ambre-spec-prose-pagination.md`) -- **none exists**, and no code either |
| Prose pagination, partial reflow of 10 pages or fewer | < 3 ms | -- | Logic | the early exit converges inside the frame; an edit whose reflow does not converge is charged to the row above -- **none exists**, and no code either |
| Prose cold shape, 10 dirty blocks | < 5 ms | -- | Logic | a glyph-level pass, an order of magnitude over placement -- **none exists**, and no code either |
| Prose cold shape at document open, about 300 blocks | < 150 ms | -- | Logic | one-shot, off the interactive path -- **none exists**, and no code either |
| Prose page-layout handoff, 100 pages | < 1 ms | -- | Logic -> UI | a typed value by move on an `ambre_bus::conduit` queue, no serialization -- **none exists**, and no code either |
| Prose `export_snapshot()`, 100 pages | < 10 ms | -- | Logic | **none exists**, and no code either |
| Prose table-of-contents build, 50 headings | < 1 ms | -- | Logic | **none exists**, and no code either |
| Prose list ordinals, 100 items | < 500 us | -- | Logic | derived at read time, never stored (`PROSE-FEAT-002`) -- **none exists**, and no code either |
| Prose heading section numbers, 50 headings | < 500 us | -- | Logic | **none exists**, and no code either |
| Prose track-changes command intercept | < 100 us | -- | Logic | on the path of every mutation while suggesting is on -- **none exists**, and no code either |
| Prose accept all, 100 suggestions | < 5 ms | -- | Logic | **none exists**, and no code either |
| Prose find, 100 pages, 10-character query | < 20 ms | -- | Logic | **none exists**, and no code either |
| Prose replace all, 50 occurrences | < 30 ms | -- | Logic | **none exists**, and no code either |
| Prose word count, 100 pages | < 5 ms | -- | Logic | debounced off the keystroke path -- **none exists**, and no code either |
| Prose spell check, 200 words | < 10 ms | -- | Logic | post-V1, and the dictionary is lazily loaded -- **none exists**, and no code either |
| Prose DOCX import / export, 100 pages | < 500 ms / < 300 ms | -- | Logic | sequential ZIP and XML work, one-shot and off the render frame -- **none exists**, and no code either |
| Prose Markdown export, 100 pages | < 50 ms | -- | Logic | no layout required -- **none exists**, and no code either |
| Prose PDF export, 100 pages, without / with images | < 2 s / < 5 s | -- | Logic | shaping per page through the shared surface; the second figure adds the blob fetches -- **none exists**, and no code either |
| Prose HTML clipboard import, 500 KB | < 100 ms | -- | Logic | on the paste path, so it is the one interop leg a user waits on -- `crates/ambre-prose/benches/html_paste.rs` measures the whole paste, desktop only; Google Docs, Word and the deepest nesting hold it, **a dense web page does not** |
| Prose plaintext `.ambre` export, 100 pages | < 10 ms | -- | Logic | **none exists**, and no code either |
| Prose document to intermediate form, 100 pages | < 50 ms | -- | Logic | one tree walk, shared by all four export formats -- **none exists**, and no code either |
| Prose scroll of one page | < 16 ms | -- | UI | culling plus the rebuild of the pages revealed (`features/ambre-spec-prose-viewport.md`) -- **none exists**, and no code either |
| Prose slash menu opened | < 8 ms | -- | UI | **none exists**, and no code either |
| Prose contextual toolbar shown | < 4 ms | -- | UI | **none exists**, and no code either |
| Keystroke to a local draft echo | < 2 ms | -- | UI | the editor echoes locally with no round trip: Trame's inline cell editor, Prose's draft buffer -- the `ambre:echo` probe on the page's timeline, read by `scripts/keystroke_probes.mjs` over single keys, a burst and a 10 KB paste; `?probes` prints its median and 95th percentile. Read by hand on a desktop only, and never on the slow devices the target names; Trame's editor is not measured |
| Keystroke to CRDT confirmation | 2 frames | -- | UI <-> Logic | counted in animation frames, not milliseconds: the first frame after the key paints the echo, and the answer is applied by the next one. The relay reads answers once a frame, so a time would measure the refresh rate, and reading sooner would move no pixel. The `ambre:confirm-frames` mark on the page's timeline, read by `scripts/keystroke_probes.mjs` over single keys, a burst and a 10 KB paste; `?probes` prints its median and 95th percentile. Read by hand on a desktop only, and never on the slow devices the target names |
| Logic pass that answers a keystroke | < 4 ms | -- | Logic | from the pass taking the command to its answer leaving, the part of the confirmation the Logic thread holds: under a frame, with room for the relay. The `ambre:logic-pass` measure on the worker's timeline, read from a Chrome trace by `scripts/keystroke_probes.mjs` over single keys. Read by hand on a desktop only, and never on the slow devices the target names |
| Keydown to a moved caret, end to end | < 50 ms | < 100 ms | UI <-> Logic | from the input handler to the end of the first frame that shows it -- the `ambre:caret` probe on the page's timeline, read by `scripts/keystroke_probes.mjs` over single keys, a burst and a 10 KB paste; `?probes` prints its median and 95th percentile. Read by hand on a desktop only, and never on the slow devices the target names |
| `ScrollViewport` send | < 1 ms | -- | UI | non-blocking -- **none exists** |
| Receive and render `TrameEvent::ViewportData` | < 5 ms | -- | UI | typed value by move, plus the vertex batch build -- **none exists** |
| Autocompletion, computation plus dropdown | < 2 ms | -- | UI | static lookup, no round trip -- **none exists** |
| Macro run started by a gesture (wall clock, network waits included) | -- | 30 s | Macro sandbox | the worker is terminated at the ceiling -- **none exists** |
| Macro run fired by an edit trigger (wall clock) | -- | 2 s | Macro sandbox | the worker is terminated at the ceiling -- **none exists** |
| Macro run fork, 10k-cell workbook | < 50 ms | -- | Logic | criterion -- **none exists** |

* **The transport is not the cost.** UI <-> Logic stays zero-cost (`ambre_bus::conduit` by move,
  shared heap). The Logic <-> Sync `postMessage` hop adds a postcard encode plus a structured clone,
  or a zero-copy `ArrayBuffer` transfer for a large payload. The relay's per-frame cadence bounds
  it, well under the < 300 ms inter-user budget, because network latency dominates that path. The
  crypto bottleneck is Ed25519, 80 to 250 us.
* **p99 over peak.** A stable 45 FPS reads as smoother than 60 FPS with stutters, so the target is
  regularity. A guaranteed 60 FPS on every mobile device is not a goal.
* **The `.ambre` container has no budget, and that is a result.** It adds a 22-byte write and a
  22-byte parse. An export is `export_snapshot()` plus a header; an import is a header parse plus
  `import_snapshot`. Both are legs 2 to 4 of section 4. Its binary contribution is
  negligible for the same reason -- a few dozen bytes in `ambre-core`, shared rather than duplicated
  per app (`ambre-spec-docfile.md section 3`).

---

## 2. THE FRAME (16.6 MS AT 60 FPS)

| Phase | Budget | Thread | Notes |
|:---|:---|:---|:---|
| Keypress -> `beforeinput` -> dispatch | ~0.1 ms | UI | Negligible |
| `AppView::update` (drain the `UiEvent` queue) | < 0.1 ms | UI | Inbound IPC, dirty flag mutations |
| Content hit-test and input dispatch (in view) | < 0.2 ms | UI | `AppView::handle_input`, pointer and keyboard on the canvas area |
| View state update | < 0.5 ms | UI | Internal state mutations, transition logic |
| `LoroText::insert` | < 0.1 ms | Logic | Sequential insert, off the render frame |
| Text layout (Parley, dirty paragraph) | < 2 ms | UI or Logic | UI-thread shaping for the in-cell editor case (Trame cells, multi-line), typical < 0.5 ms. **Prose shapes on the UI thread the paragraphs an edit in flight changes, and no more**: each one whole, until the layout that answers carries it, and the input method's preview (`features/ambre-spec-prose-text-editing.md section 3`). Pagination shapes in the Logic thread and ships pre-shaped `ParleyRunData`, so this row is a paragraph or two per edit on the UI thread for Prose. `prose_paragraph_shape` in `crates/ambre-logic/benches/scale.rs` measures one paragraph natively. The budget holds up to 2 000 characters, a long paragraph of prose; a longer one costs what it costs, and nothing is degraded to save it (`PROSE-UI-215`) and 0.5-2 ms on the Logic thread (`features/ambre-spec-prose-pagination.md`, `features/ambre-spec-prose-page-rendering.md`) |
| Content accessibility diff (AccessKit) | < 0.2 ms | UI | OS notifications on the delta only |
| Glyph rasterization (Swash) on a new glyph | ~0 ms nominal | UI | Atlas hit |
| Build the vertex batch (dirty only) | < 1 ms | UI | `QuadRenderer::begin_frame` plus the scene draw. Zero allocation in steady state |
| GPU upload and render pass | < 1 ms | UI | `write_buffer` plus up to 3 draw calls per layer (quads, mask glyphs, color glyphs). Typical 0.3-0.5 ms |
| Antialiasing band, share of a tessellated path | < 15 % of its vertices, < 5 % of the batch build | UI | The silhouette band of `ambre-spec-drawing-surface.md section 2` (`CANVAS-061`). It adds vertices to a buffer and a draw call that already exist, so it is charged against the two rows above and against nothing else: no pass, no dispatch, no attachment (`CANVAS-062`) -- **none exists** |
| Toile board paint, 2 000 visible elements | < 16 ms | UI | The whole frame for a busy board, and the reason the cull is the engine's job (`features/ambre-spec-toile-board-rendering.md`) |
| Toile pan and zoom, input to paint | < 15 ms | UI | The camera is applied locally, so the gesture never waits on the Logic thread (`features/ambre-spec-toile-viewport.md`) |
| Toile presence overlay, 20 cursors and their trails | < 1 ms | UI | Instanced quads and small ring buffers (`features/ambre-spec-toile-presence.md`) |
| Prose display list, 3 visible pages | < 4 ms | UI | Walking the placed blocks and assembling already shaped runs, with the highlights costing what is visible (`features/ambre-spec-prose-page-rendering.md`) |
| Prose page paint, 3 visible pages | < 14 ms | UI | The display list above, the vertex batch and upload (< 4 ms) and the submit (< 6 ms). A full page repaint is the busy case, as the board paint is for Toile |
| **Total target** | **< 5 ms typical** | UI | 11 ms of margin for a complex case |
| `TextCache`, 100 distinct layouts, cold cache | < 1 ms | UI | Not a frame phase: the shape budget of the cache itself, off the critical path once warm |

* **The scroll round-trip holds because the frame is a fifth of it.** Section 1 budgets the P1 scroll
  round-trip at < 15 ms. Rendering consumes < 5 ms. Queue transport is negligible: < 1 us per
  hop, no serialization, no boundary memcpy. Most of the budget is left to the Logic thread.
* **Threats to the frame, and the countermeasure each takes.**
  1. **Parley has no incremental layout** -> atomicity at the **paragraph**, one `Layout` per
     paragraph. For Prose this shaping runs in the Logic thread, and a monster paragraph is split
     internally and deferred through the P1-P4 priority scheduler
     (`4-Prose/features/ambre-spec-prose-pagination.md`). That scheduler is **not**
     `requestIdleCallback`, which does not exist in the `wasm_thread` Logic worker. It stays
     available for opportunistic
     UI-thread work.
  2. **Glyph atlas overflow** -> an LRU per 256^2 / 512^2 slot, one cache per size. UI thread.
  3. **Scrolling a long document** -> per-paragraph virtualization. Render the visible range plus a
     buffer, cache the pre-shaped runs in the UI `RunCache`, and cull by page for Prose
     (`features/ambre-spec-prose-viewport.md`).
  4. **A multi-paragraph selection** -> reuse the cached `Layout` to extract the rectangles.
  5. **A mid-range mobile device** -> wgpu quads hold 60 FPS; the risk is CPU (Parley, Loro and
     dispatch share 6 to 8 usable ms).

---

## 3. THE SERVER, PER OPERATION

| Operation | Target | Potential bottleneck | Method |
|---|---|---|---|
| WS `Update` receive and broadcast | < 5 ms p99 | Redis Lua plus PubSub | **none exists** |
| Redis Lua clock validation | < 1 ms | Redis round trip | **none exists** |
| `GET` deltas, 100 entries | < 20 ms | Postgres read | **none exists** |
| S3 presigned URL generation | < 50 ms | S3 SDK network I/O | **none exists** |
| Document cold start (snapshot plus N deltas) | < 500 ms | S3 plus Postgres read | **none exists** |
| OIDC auth callback | < 200 ms | Provider token exchange | **none exists** |
| Global S3 flush batch, 15 s cycle | < 2 s for 1000 deltas | Parallel S3 upload | **none exists** |
| Presence write, per heartbeat | < 2 ms | Two Redis round trips: the entry, then its own expiry | **none exists** |

---

## 4. OPENING A LARGE DOCUMENT -- A COMPOSITE OPERATION WITH NO END-TO-END TARGET

* **Every leg is budgeted, the whole is not.** Each row of section 1 budgets one leg; none budgets
  the chain, and two legs are unbudgeted entirely.

| Leg of an open | Stated budget |
|---|---|
| 1. Server read (snapshot plus N deltas from S3 and Postgres) | < 500 ms, section 3 |
| 2. Client AEAD open plus zstd decompress of a full snapshot | **none** -- section 1 budgets compression of 100 KB and inbound decryption per 1 KB |
| 3. `LoroDoc` import of a full snapshot | **none** -- `doc.import` in section 1 is < 2 ms for a **1 KB delta**, which says nothing about a 10 MB snapshot. A full-state read imports it **twice**: once into a scratch document to read the schema register, once into the live one (`ambre-spec-schema-version.md section 2`). Deliberate, and it is what makes a refused document cost the user nothing |
| 4. Engine rebuild after import | partial -- `SheetIndex` rebuild of 10k rows is < 10 ms in section 1. Trame's `import_snapshot` triggers the **full reseed**, which re-parses and recompiles every formula of every sheet (`ambre-spec-trame-engine.md`): unbudgeted, and likely dominant at 100k cells. Toile's `SpatialIndex` **bulk load** is unbudgeted too |
| 5. First painted frame | the frame budget of section 2, GPU-bound |

> **This section sets the missing figure, once it can be measured.** Legs 2 to 5 are pure functions
> of a snapshot, so a large document makes them measurable on host, with no network and no browser.
> Leg 1 needs the db tier. `ambre-spec-fixtures.md` (`FIXTURES P2`) builds the
> corpus and runs those benches. Inventing an "open in < X ms" target before then would produce a
> number with no basis. This is recorded as a **known gap with an owner**, not as a budget.

---

## 5. BINARY SIZE

* **Two binaries.** `ambre-app` carries UI plus Logic: wgpu, loro, leptos, the heavy one.
  `ambre-sync` carries crypto plus protocol plus gloo-net, small at ~6.8 MB unoptimized, with no
  wgpu, no loro and no leptos. Only crypto, protocol and serde are duplicated across the two; the heavy
  dependencies live once, in `ambre-app`. The two wasm fetches are parallelizable. **Total Main
  budget < 2 MB gzip**, and Prose interop stays **lazy-loaded**.

| Contribution | Estimated raw | Notes |
|---|---|---|
| `usvg` plus `lyon` (build-time icons) | ~80 KB | Icon pipeline, precompiled vertex buffers |
| `swash` | ~120 KB | Glyph rasterization |
| Fonts (Noto Sans plus Mono) | 0 KB binary | ~600 KB fetched and HTTP-cached: Regular plus Bold, Latin, Cyrillic, Greek |
| Icon atlas blob (vertex buffers) | < 200 KB | ~500 tessellated icons |
| `image` (PNG, JPEG, WebP) | ~60 KB | Feature-flagged, only the activated decoders |
| `ambre-sync` logic | ~50 KB | Excluding its dependencies |
| RustCrypto (chacha20, ed25519, blake2, hmac, argon2) | ~140 KB | Shared with `ambre-crypto`; Argon2 is there for login, and for a passphrase-sealed file, which reuses the same work factor rather than adding a second one (`ambre-spec-docfile.md`) |
| `ruzstd` (compression and decompression) | ~80 KB | |
| `gloo-net` (websocket and http) | ~40 KB | |
| `postcard` | ~15 KB | Shared |
| **`ambre-sync` total** | **~325 KB raw** | Shared dependencies deduplicate in the final binary |

**The Main instance, per contribution.** Estimates, and why the lazy-loading rule above exists.

| Contribution | Estimated raw | Estimated gzip | Notes |
|---|---|---|---|
| `ambre-prose`, engine plus features plus interop plus UI | ~230 KB | ~72 KB | Of which the UI module is ~50 KB raw |
| `unicode-segmentation` | ~40 KB | ~15 KB | Word and grapheme boundaries: the word count, caret movement |
| `zspell` plus one dictionary | ~550 KB | ~210 KB | Post-V1, and lazily loaded on the first spell check |
| `docx-rs` | ~300 KB | ~90 KB | Lazily loaded on the first DOCX import or export |
| `html5ever` plus `markup5ever_rcdom` | ~200 KB | ~60 KB | Lazily loaded on the first HTML import |
| `pulldown-cmark` and its writer | ~80 KB | ~25 KB | Lazily loaded |
| `krilla`, billed to `ambre-pdf` | ~400 KB | ~120 KB | Counted once. Toile's deck export is the first consumer to link it: the launcher enables `ambre-logic/toile-pdf`, so the writer is in the Main instance from that build on, and Prose's export adds none of it |
| `loro` | ~400 KB | ~120 KB | Shared by every application |
| `parley` | ~200 KB | ~60 KB | Shared |
| `wgpu` plus `lyon` plus `swash` | ~800 KB | ~250 KB | Shared, and the `swash` row above is its glyph half alone |

* **The heavy render crates live in the Main instance.** wgpu, lyon and swash link once there,
  deduplicated with the rest; the separate Sync instance links none of them.
* **Fixture machinery contributes zero** (`FIXTURE-007`): it is host-only and gated behind a
  non-default `fixtures` feature that `ambre-app` never names.

---

## 6. MEMORY

* **Two independent** `WebAssembly.Memory` heaps -- Main shared across UI and Logic, Sync its own --
  each capped at 4 GiB by the platform.

| Holder | Budget | Notes |
|:---|:---|:---|
| CPU vertex buffers (`QuadRenderer`) | < 5 MB | Proportional to the visible draw-call count |
| wgpu textures and buffers | < 20 MB | Swap chain, staging buffers, `GlyphAtlas` (2048x2048 R8 = 4 MB). The RGBA color atlas (2048x2048 = 16 MB) is **allocated lazily** on the first color glyph -- a 1x1 placeholder otherwise -- so an emoji-free document pays nothing |
| Uploaded pictures, mip chains included | < 24 MB | `IMAGE_VRAM_BUDGET`, charged apart from the row above rather than sharing an unstated remainder of it. At four bytes a texel that is roughly six 1024x1024 photographs, each carrying a chain a third its own size. Held by the eviction (`CANVAS-066`) and measured by nothing: the frame where the budget starts to thrash -- evicting a picture the next frame asks for again -- is the figure no bench reads (`ambre-spec-drawing-surface.md section 5`) |
| View state (content) | < 5 MB | Proportional to the visible content: cells, text runs |
| Fonts in memory | < 10 MB | Base fonts plus lazily loaded ones |
| Icon atlas | < 1 MB | Compiled vector blob |
| **Total UI thread** | **< 41 MB** | Within the global < 100 MB |
| `SyncState` base, per session | < 1 MB | `MasterSeed`, `SigningKey`, `ExchangeKey`, plus `HashMap` overhead |
| `SyncDocumentState`, per open document | < 1 MB | `DocumentKey`, counters, sync state, quarantine |
| `quarantine`, per document | ~2 MB max | Capped at 1000 updates of ~2 KB average |
| `old_keys`, per document | negligible | Capped at 10 `DocumentKey` (32 bytes plus overhead) |
| Unconfirmed updates, per document | 1024 updates | `UNCONFIRMED_LIMIT`: the sent frames the relay has not echoed yet, kept to resend one it refuses (`SYNC-129`). Past the cap the oldest is forgotten, and a refusal for it resends nothing |
| Sync events held on Main past a full channel | 4096 events, released at 1024 | `SYNC_INBOX_HIGH` and `SYNC_INBOX_LOW`, four times and once the channel capacity. At the high mark the Sync worker closes its socket, and the relay holds the rest (`SYNC-059`). The bound is a count, not bytes: a replay batch is one event |
| Sync overhead, 1 host document plus 3 embedded | ~4-8 MB | The typical scenario |
| `LoroDoc` | < 10 MB target, per 100k cells | |
| `LoroDoc` Prose | < 10 MB, per 100 pages and 50 000 words | |
| Prose page layout, 100 pages | < 200 KB | About 300 blocks, 20 footnotes and the header and footer runs. Derived, so it is re-allocated whole on a full recompute |
| Prose shaped runs, Logic thread | ~150 KB per page, unbounded | One run per block, kept past measurement and shared with the view (`PROSE-036`). Measured at about 18 KB a paragraph of eighty words, eight to a page: 43 MB for 300 pages. Bounding it is the `Shaped runs bounded to the view` Story (`ambre-roadmap.md` E7) |
| Prose run cache, 500 runs | < 1 MB | The drawing state of the runs painted lately, about 50 pages: an index into runs the Logic thread already holds (`features/ambre-spec-prose-page-rendering.md`) |
| Prose view state, excluding the run cache | < 1 MB | |
| Prose decoded pictures, per session | < 64 MiB | A bounded cache in the Logic thread, evicted least recently painted; it holds the pictures of a few pages and stays well inside the < 100 MB total (`features/ambre-spec-prose-images.md`) |
| Prose vertex buffers and atlases, 3 pages | < 50 MB VRAM | Stated by the Prose design and larger than the shared wgpu row above, which is the arithmetic the two rows now make visible. Reconciled by the `The memory budgets add up` Story (`ambre-roadmap.md` E7) |
| Trame DAG plus cache | < 50 MB, per 100k formula cells | |
| Toile board plus index, 50 000 elements | proportional to the element count, inside the < 100 MB total | The R-tree adds a small multiple of its leaf count over the bounds map it mirrors |
| Server, per open document | ~50 KB | No in-process blob cache: `LIST doc:{id}:pending` buffers in Redis, ~50 deltas of ~1 KB over the 15 s window |
| Server, inline delta size | < 64 KB | A delta past it keeps an object of its own (`SRV-149`) |
| Server, inline deltas per document | 256 deltas or 256 KB, whichever comes first | Past it, the oldest move into one batch object (`SRV-150`, `SRV-151`). Bounds the register at the documents in use times this figure |
| Documents held in memory, per client | < 32 MB, to measure | Every document the Logic thread holds for a list with activity badges, pinned ones included. Held by eviction (`RESID-001`) |
| Local store, per client | < 500 MB, to measure | Inside the browser quota. Held by eviction (`RESID-002`) |
| Murmure segment, at its close | 5000 messages or 2 MB of snapshot, whichever comes first | Bounds the first display of a channel and every final snapshot. Measured with chat-length bodies: 5000 messages make a snapshot of 879 KB, 581 KB shallow, so the message count binds first |
| Murmure members before an `@channel` asks for confirmation | 10 | Past it, one `@channel` notifies enough people to warrant a second thought. A first estimate, to measure against use (`MURMURE-231`) |
| Murmure members past which a channel defaults to the mentions level | 30 | A busy room would otherwise wake every member for every line. A first estimate (`MURMURE-267`) |
| Murmure pins per channel | 50 | A pinned list longer than a screen stops being a list of what matters (`MURMURE-339`) |
| Murmure custom emoji, per space | 500, of 128 KB each at most | Every member loads the images the channel shows, so the cap bounds the space document and the images drawn |
| Murmure active segment, deltas past its latest snapshot | < 250, to measure on a phone | A cold open pays one inbound open (section 1, < 350 us) and one replay (section 1, < 250 us) for each. 250 of them cost about 150 ms of a phone cold open. Without a snapshot, a segment at its close replays 6250 deltas, fetched as 6250 objects. The relay asks for the snapshot (`SRV-038`). The phone figure assumes a phone about 7 times slower than the desktop: the ratio of the inbound open budget (350 us) to its desktop measure (50 us). The `Murmure measured on a phone` Story (`ambre-roadmap.md` E5) replaces that ratio with a measure |
| Light subscription, per document not held | < 1 KB | The key, the counters and the cursor (`RESID-004`) |
| User settings document | < 256 KB, to measure | Pinned while the user is signed in (`SETTINGS-005`), and counted in the documents held in memory |
| Color emoji font | loaded on first use, size to measure | Charged to the fonts row above. A full color emoji font weighs about 10 MB, so its size decides whether the fonts row still holds |

---

## 7. TOOLING

| Recipe | Target | Notes |
|---|---|---|
| `just fixtures` | ~30 s | A slow corpus is a corpus developers stop regenerating |
| `just fixtures-check` in CI | ~10 s | Holds only because the frozen set is capped at three per `doc_type`, and the check loads and migrates every one |
| Committed fixture artifact | 256 KB each, ~5 MB total | Indicative cap over `fixtures/current/` and `fixtures/frozen/`. The scale corpora are git-ignored and produced on demand (`ambre-spec-fixtures.md section 2`) |

---

## 8. POLICY DURATIONS

A duration a rule waits on before it acts. Each figure is a first estimate, to measure.

| Duration | Target | Why this order of size | Method |
|---|---|---|---|
| Grace period before a closing Murmure segment is finalized | 24 h | a typo fix lands within minutes, so a day keeps every recent edit out of a sealed segment | **none exists** |
| Murmure message grouping span | 5 min | consecutive messages of one author inside the span share one header, as every chat product groups them | **none exists** |
| Finalize lease | 30 s | long enough to encode, seal and upload one segment of about 2 MB | **none exists** |
| Idle delay before a document's inline deltas move to a batch object | 24 h | a document untouched for a day is rarely opened within the next, and its tail then costs one object read | **none exists** |
| Delay between a Murmure delete after the finalization and its erasure | < 30 days | the re-finalization runs in batches, and an erasure request expects an answer within a month | **none exists** |
