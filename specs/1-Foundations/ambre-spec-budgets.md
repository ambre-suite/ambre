# AMBRE -- THE BUDGET REGISTER

**Type:** Register (every budget of the workspace)

> Every performance and resource target Ambre commits to, one row each. A rule that must hold a
> budget cites the row here instead of restating the figure.

* **Every budget of the workspace is a row here.** A budget a reader cannot compare to its
  neighbour is a number, not a target, so no component spec carries one of its own: a rule that must
  hold a budget cites the row instead (`CANVAS-005`, `FIXTURE-007`). `Method` names the mechanism
  that measures the row, and "**none exists**" is the honest value.
* **This file is a register, not a specification.** It states no rule of its own, so it declares no
  contract table: `ambre-spec-template.md section 4` holds what a file of this class owes.

---

## 1. THE CLIENT, PER OPERATION (TARGET: MOBILE HARDWARE 2024)

| Operation | Target (P95 mobile) | Ceiling | Thread | Method |
|-----------|-------------------|---------|--------|---------|
| `flume` send/receive UI <-> Logic (shared heap, by move) | < 1 us | -- | UI <-> Logic | benchmark |
| Logic <-> Sync `postMessage` (postcard encode + structured clone / transfer) | < 1 ms | -- | Main <-> Sync | benchmark |
| Scroll round-trip UI -> Logic -> UI (P1) | < 2 ms | -- | UI <-> Logic | benchmark e2e |
| Outbound encryption 1 KB (compress+AEAD+HMAC+Ed25519) | < 200 us | < 500 us | Sync | criterion |
| Inbound decryption 1 KB (verify+HMAC+AEAD+decompress) | < 350 us | < 1 ms | Sync | criterion |
| Full crypto round-trip (excluding network) | < 600 us | < 1.5 ms | Sync | criterion |
| Snapshot compression 100 KB (ruzstd) | < 5 ms | -- | Sync | criterion |
| Quarantine release, 100 updates | < 1 ms | -- | Sync | criterion -- **none exists** |
| `doc.import()` Loro update (1 KB delta) | < 2 ms | -- | Logic | criterion |
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
| Toile slide transition, 500 elements per frame | < 4 ms | -- | Logic | leaves room to paint inside the frame during a transition -- **none exists** |
| Toile layout solve, 200 children | < 1 ms | -- | Logic | runs at derivation on every child edit -- **none exists**, and nothing calls the solver (`TOILE-FEAT-029`) |
| Toile reaction tally | < 0.1 ms typical | -- | Logic | one scan of the reactions map on read, never stored -- **none exists** |
| Toile boolean combination, two typical paths | < 5 ms | -- | Logic | user-initiated and one-shot, off the render frame -- **none exists** |
| Toile SVG import, 5 000 nodes | < 500 ms | -- | Logic | one-shot user action, off the render thread -- **none exists** |
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
| Prose page-layout handoff, 100 pages | < 1 ms | -- | Logic -> UI | a typed value by move on `flume`, no serialization -- **none exists**, and no code either |
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
| Prose HTML clipboard import, 500 KB | < 100 ms | -- | Logic | on the paste path, so it is the one interop leg a user waits on -- **none exists**, and no code either |
| Prose plaintext `.ambre` export, 100 pages | < 10 ms | -- | Logic | **none exists**, and no code either |
| Prose document to intermediate form, 100 pages | < 50 ms | -- | Logic | one tree walk, shared by all four export formats -- **none exists**, and no code either |
| Prose scroll of one page | < 16 ms | -- | UI | culling plus the rebuild of the pages revealed (`features/ambre-spec-prose-viewport.md`) -- **none exists**, and no code either |
| Prose slash menu opened | < 8 ms | -- | UI | **none exists**, and no code either |
| Prose contextual toolbar shown | < 4 ms | -- | UI | **none exists**, and no code either |
| Keystroke to a local draft echo | < 2 ms | -- | UI | the editor echoes locally with no round trip: Trame's inline cell editor, Prose's draft buffer -- **none exists** |
| Keystroke to CRDT confirmation | < 15 ms | -- | UI <-> Logic | the round trip Prose budgets too -- **none exists** |
| Keydown to a moved caret, end to end | < 50 ms | < 100 ms | UI <-> Logic | `performance.mark` and `performance.measure`, on desktop **and** mobile, over a single keystroke, a sustained typing burst and a 10 KB paste -- **none exists** |
| `ScrollViewport` send | < 1 ms | -- | UI | non-blocking -- **none exists** |
| Receive and render `TrameEvent::ViewportData` | < 5 ms | -- | UI | typed value by move, plus the vertex batch build -- **none exists** |
| Autocompletion, computation plus dropdown | < 2 ms | -- | UI | static lookup, no round trip -- **none exists** |

* **The transport is not the cost.** UI <-> Logic stays zero-cost (`flume` by move, shared heap). The
  Logic <-> Sync `postMessage` hop adds a postcard encode plus a structured clone, or a zero-copy
  `ArrayBuffer` transfer for a large payload -- bounded by the relay's per-frame cadence, well under
  the < 300 ms inter-user budget, because network latency dominates that path. The crypto bottleneck
  is Ed25519, 80 to 250 us.
* **p99 over peak.** A stable 45 FPS reads as smoother than 60 FPS with stutters, so the target is
  regularity. A guaranteed 60 FPS on every mobile device is not a goal.
* **The `.ambre` container has no budget, and that is a result.** It adds a 22-byte write and a
  22-byte parse: an export is `export_snapshot()` plus a header, an import is a header parse plus
  `import_snapshot`, and both of those legs are legs 2 to 4 of section 4. Its binary contribution is
  negligible for the same reason -- a few dozen bytes in `ambre-core`, shared rather than duplicated
  per app (`ambre-spec-docfile.md section 3`).

---

## 2. THE FRAME (16.6 MS AT 60 FPS)

| Phase | Budget | Thread | Notes |
|:---|:---|:---|:---|
| Keypress -> `beforeinput` -> dispatch | ~0.1 ms | UI | Negligible |
| `AppView::update` (drain the `flume` `UiEvent` queue) | < 0.1 ms | UI | Inbound IPC, dirty flag mutations |
| Content hit-test and input dispatch (in view) | < 0.2 ms | UI | `AppView::handle_input`, pointer and keyboard on the canvas area |
| View state update | < 0.5 ms | UI | Internal state mutations, transition logic |
| `LoroText::insert` | < 0.1 ms | Logic | Sequential insert, off the render frame |
| Text layout (Parley, dirty paragraph) | < 2 ms | UI or Logic | UI-thread shaping for the in-cell editor case (Trame cells, multi-line), typical < 0.5 ms. **Prose does NOT shape on the UI thread**: pagination shapes in the Logic thread and ships pre-shaped `ParleyRunData`, so this row is ~0 on the UI thread for Prose and 0.5-2 ms on the Logic thread (`features/ambre-spec-prose-pagination.md`, `features/ambre-spec-prose-page-rendering.md`) |
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

* **The scroll round-trip holds because the frame is a fifth of it.** The P1 scroll round-trip is
  budgeted at < 15 ms in section 1; rendering consumes < 5 ms and `flume` transport is negligible
  (< 1 us per hop, no serialization, no boundary memcpy), which leaves most of the budget to the
  Logic thread.
* **Threats to the frame, and the countermeasure each takes.**
  1. **Parley has no incremental layout** -> atomicity at the **paragraph**, one `Layout` per
     paragraph. For Prose this shaping runs in the Logic thread, and a monster paragraph is split
     internally and deferred through the P1-P4 priority scheduler
     (`4-Prose/features/ambre-spec-prose-pagination.md`) -- **not** `requestIdleCallback`, which
     does not exist in the `wasm_thread` Logic worker. It stays available for opportunistic
     UI-thread work.
  2. **Glyph atlas overflow** -> an LRU per 256^2 / 512^2 slot, one cache per size. UI thread.
  3. **Scrolling a long document** -> per-paragraph virtualization: render the visible range plus a
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
| 3. `LoroDoc` import of a full snapshot | **none** -- `doc.import` in section 1 is < 2 ms for a **1 KB delta**, which says nothing about a 10 MB snapshot |
| 4. Engine rebuild after import | partial -- `SheetIndex` rebuild of 10k rows is < 10 ms in section 1. Trame's `import_snapshot` triggers the **full reseed**, which re-parses and recompiles every formula of every sheet (`ambre-spec-trame-engine.md`): unbudgeted, and likely dominant at 100k cells. Toile's `SpatialIndex` **bulk load** is unbudgeted too |
| 5. First painted frame | the frame budget of section 2, GPU-bound |

> **This section sets the missing figure, once it can be measured.** Legs 2 to 5 are pure functions
> of a snapshot, so they are measurable on host with no network and no browser as soon as a large
> document exists; leg 1 needs the db tier. `ambre-spec-fixtures.md` (`FIXTURES P2`) builds the
> corpus and runs those benches. Inventing an "open in < X ms" target before then would produce a
> number with no basis, so this is recorded as a **known gap with an owner**, not as a budget.

---

## 5. BINARY SIZE

* **Two binaries.** `ambre-app` carries UI plus Logic (wgpu, loro, leptos -- the heavy one) and
  `ambre-sync` carries crypto plus protocol plus gloo-net (small, ~6.8 MB unoptimized, no wgpu, no
  loro, no leptos). Only crypto, protocol and serde are duplicated across the two; the heavy
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
| RustCrypto (chacha20, ed25519, blake2, hmac, argon2) | ~140 KB | Shared with `ambre-crypto`; Argon2 is there for login |
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
| Sync overhead, 1 host document plus 3 embedded | ~4-8 MB | The typical scenario |
| `LoroDoc` | < 10 MB target, per 100k cells | |
| `LoroDoc` Prose | < 10 MB, per 100 pages and 50 000 words | |
| Prose page layout, 100 pages | < 200 KB | About 300 blocks, 20 footnotes and the header and footer runs. Derived, so it is re-allocated whole on a full recompute |
| Prose transient shaped layouts, during measurement | peak < 5 MB | Released once placement is done: only a block that splits keeps its full layout (`PROSE-036`) |
| Prose run cache, 500 blocks | < 20 MB | Shaped runs plus their glyph state, about 50 pages (`features/ambre-spec-prose-page-rendering.md`) |
| Prose view state, excluding the run cache | < 1 MB | |
| Prose decoded pictures, per session | < 64 MiB | A bounded cache in the Logic thread, evicted least recently painted; it holds the pictures of a few pages and stays well inside the < 100 MB total (`features/ambre-spec-prose-images.md`) |
| Prose vertex buffers and atlases, 3 pages | < 50 MB VRAM | Stated by the Prose design and larger than the shared wgpu row above, which is the arithmetic the two rows now make visible |
| Trame DAG plus cache | < 50 MB, per 100k formula cells | |
| Toile board plus index, 50 000 elements | proportional to the element count, inside the < 100 MB total | The R-tree adds a small multiple of its leaf count over the bounds map it mirrors |
| Server, per open document | ~50 KB | No in-process blob cache: `LIST doc:{id}:pending` buffers in Redis, ~50 deltas of ~1 KB over the 15 s window |

---

## 7. TOOLING

| Recipe | Target | Notes |
|---|---|---|
| `just fixtures` | ~30 s | A slow corpus is a corpus developers stop regenerating |
| `just fixtures-check` in CI | ~10 s | Holds only because the frozen set is capped at three per `doc_type`, and the check loads and migrates every one |
| Committed fixture artifact | 256 KB each, ~5 MB total | Indicative cap over `fixtures/current/` and `fixtures/frozen/`. The scale corpora are git-ignored and produced on demand (`ambre-spec-fixtures.md section 2`) |
